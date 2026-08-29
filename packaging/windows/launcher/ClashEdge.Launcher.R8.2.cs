using System;
using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows.Forms;

internal static class ClashEdgeLauncher
{
    // --- Junction target resolution (audit P1-6) ------------------------------

    private const int FsctlGetReparsePoint = 0x000900A8;
    private const uint IoReparseTagMountPoint = 0xA0000003;

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr CreateFile(
        string lpFileName,
        uint dwDesiredAccess,
        uint dwShareMode,
        IntPtr lpSecurityAttributes,
        uint dwCreationDisposition,
        uint dwFlagsAndAttributes,
        IntPtr hTemplateFile);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr hObject);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool FlushFileBuffers(IntPtr hFile);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool DeviceIoControl(
        IntPtr hDevice,
        int dwIoControlCode,
        IntPtr lpInBuffer,
        int nInBufferSize,
        byte[] lpOutBuffer,
        int nOutBufferSize,
        out int lpBytesReturned,
        IntPtr lpOverlapped);

    private static string GetJunctionTarget(string path)
    {
        IntPtr handle = CreateFile(path, 0, 7, IntPtr.Zero, 3,
            0x02000000 | 0x00200000, IntPtr.Zero);
        if (handle.ToInt64() == -1) return null;
        try
        {
            var buffer = new byte[16 * 1024];
            int returned;
            if (!DeviceIoControl(handle, FsctlGetReparsePoint, IntPtr.Zero, 0,
                    buffer, buffer.Length, out returned, IntPtr.Zero))
                return null;
            uint tag = BitConverter.ToUInt32(buffer, 0);
            if (tag != IoReparseTagMountPoint) return null;
            ushort substituteOffset = BitConverter.ToUInt16(buffer, 8);
            ushort substituteLength = BitConverter.ToUInt16(buffer, 10);
            int start = 16 + substituteOffset;
            if (start + substituteLength > returned) return null;
            return Encoding.Unicode.GetString(buffer, start, substituteLength);
        }
        finally { CloseHandle(handle); }
    }

    private static string NormalizeJunctionPath(string path)
    {
        if (path == null) return null;
        if (path.StartsWith(@"\??\", StringComparison.Ordinal)) path = path.Substring(4);
        else if (path.StartsWith(@"\\?\", StringComparison.Ordinal)) path = path.Substring(4);
        try { return Path.GetFullPath(path).TrimEnd(Path.DirectorySeparatorChar); }
        catch { return path.TrimEnd(Path.DirectorySeparatorChar); }
    }

    private static void ValidateJunctionTarget(string appDataDirectory, string portableDataDirectory)
    {
        string actual = NormalizeJunctionPath(GetJunctionTarget(appDataDirectory));
        string expected = NormalizeJunctionPath(portableDataDirectory);
        bool match = actual != null && string.Equals(actual, expected, StringComparison.OrdinalIgnoreCase);
        if (!match)
        {
            throw new InvalidOperationException(
                "数据目录联接校验失败。\n预期: " + (expected ?? "<未知>") +
                "\n实际: " + (actual ?? "<解析失败>") +
                "\n请删除 App\\ClashEdge\\data 后重新运行。");
        }
    }

    private static void CopyMissing(string source, string destination)
    {
        if (!Directory.Exists(source)) return;
        Directory.CreateDirectory(destination);
        foreach (var directory in Directory.GetDirectories(source, "*", SearchOption.AllDirectories))
            Directory.CreateDirectory(directory.Replace(source, destination));
        foreach (var file in Directory.GetFiles(source, "*", SearchOption.AllDirectories))
        {
            var target = file.Replace(source, destination);
            Directory.CreateDirectory(Path.GetDirectoryName(target));
            if (!File.Exists(target)) File.Copy(file, target);
        }
    }

    private static bool PathExists(string path)
    {
        try { File.GetAttributes(path); return true; }
        catch { return false; }
    }

    private static void MoveContents(string sourceDir, string destinationDir)
    {
        Directory.CreateDirectory(destinationDir);
        foreach (var file in Directory.GetFiles(sourceDir, "*", SearchOption.AllDirectories))
        {
            var target = file.Replace(sourceDir, destinationDir);
            Directory.CreateDirectory(Path.GetDirectoryName(target));
            if (File.Exists(target))
                File.Delete(file);
            else
                File.Move(file, target);
        }
        foreach (var dir in Directory.GetDirectories(sourceDir, "*", SearchOption.AllDirectories)
                     .OrderByDescending(d => d.Length))
        {
            if (Directory.GetFiles(dir, "*", SearchOption.AllDirectories).Length == 0)
                try { Directory.Delete(dir, true); } catch { }
        }
    }

    private static void EnsureDataJunction(string appDataDirectory, string portableDataDirectory)
    {
        if (PathExists(appDataDirectory))
        {
            var attrs = File.GetAttributes(appDataDirectory);
            if ((attrs & FileAttributes.ReparsePoint) != 0)
            {
                if (Directory.Exists(appDataDirectory))
                {
                    ValidateJunctionTarget(appDataDirectory, portableDataDirectory);
                    return;
                }
                File.Delete(appDataDirectory);
            }
            else
            {
                MoveContents(appDataDirectory, portableDataDirectory);
                Directory.Delete(appDataDirectory, true);
            }
        }
        CreateJunction(appDataDirectory, portableDataDirectory);
    }

    private static void CreateJunction(string appDataDirectory, string portableDataDirectory)
    {
        Directory.CreateDirectory(portableDataDirectory);
        var shell = Environment.GetEnvironmentVariable("ComSpec") ?? "cmd.exe";
        using (var link = Process.Start(new ProcessStartInfo(shell, "/d /c mklink /J \"" + appDataDirectory + "\" \"" + portableDataDirectory + "\"") { UseShellExecute = false, CreateNoWindow = true }))
        {
            if (link == null) throw new InvalidOperationException("无法创建便携数据目录联接。");
            link.WaitForExit();
            if (link.ExitCode != 0 || !Directory.Exists(appDataDirectory)) throw new InvalidOperationException("无法创建便携数据目录联接。");
        }
    }

    private static string Quote(string value)
    {
        return "\"" + value.Replace("\\\"", "\\\\\"") + "\"";
    }

    // --- Portable Updater (P0: transaction-safe断电恢复) ---------------------
    //
    // 状态机：
    //   pending     暂存区有 pending.json，尚未校验
    //   verified    ZIP SHA256 复验通过、解压结构校验通过
    //   swapping    新 App 已复制到 App.new-xxx 并验证完整，
    //               journal 已持久化——下一步是原子目录交换
    //   committed   交换完成、备份已删除
    //   launcher-stage       新 launcher 已复制为 ClashEdge.exe.new-<txid>，
    //                        根 exe 未被触碰
    //   launcher-swap-old    journal 记录 launcher_old/launcher_new 后，
    //                        根 exe 可能已改名走（.old-<txid> 存在）
    //   launcher-commit      新 launcher 已 rename 到位，仅剩清理
    //
    // 恢复语义：
    //   pending / verified / committed → App 未被动过或已完成，清暂存即可
    //   swapping → App 有效则交换已完成（删备份）；App 无效则从备份恢复
    //   launcher-* → 必须先于 ClashEdge.exe.old-* 全局清理处理，
    //               保证任何断电点之后至少存在一个可启动 launcher
    //
    // 硬约束：
    //   - journal 必须在破坏性操作之前持久化（写失败即中止）
    //   - 进入 swapping 后任何错误必须保证：完成新版或恢复旧版
    //   - Data/ 永远不被更新过程替换

    private static string UpdateJournalPath(string root)
    {
        return Path.Combine(root, "Data", "update-journal.json");
    }

    /// 原子写 journal（写穿持久化）。写失败必须抛出——进入 swapping /
    /// launcher-* 后 journal 是恢复链的唯一依据，不能 catch{} 静默吞掉。
    private static void WriteUpdateJournal(string root, string state)
    {
        WriteUpdateJournal(root, state, null, null, null);
    }

    /// 带 launcher 自更新字段的 journal 写入。
    /// launcher_old / launcher_new / launcher_sha 由 ExtractJsonField 解析。
    private static void WriteUpdateJournal(string root, string state,
        string launcherOld, string launcherNew, string launcherSha)
    {
        string path = UpdateJournalPath(root);
        Directory.CreateDirectory(Path.GetDirectoryName(path));
        string tmp = path + ".tmp";

        var sb = new StringBuilder();
        sb.Append("{\n  \"state\": \"").Append(state).Append("\"");
        sb.Append(",\n  \"time\": \"").Append(DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss")).Append("\"");
        if (launcherOld != null)
            sb.Append(",\n  \"launcher_old\": \"").Append(launcherOld).Append("\"");
        if (launcherNew != null)
            sb.Append(",\n  \"launcher_new\": \"").Append(launcherNew).Append("\"");
        if (launcherSha != null)
            sb.Append(",\n  \"launcher_sha\": \"").Append(launcherSha).Append("\"");
        sb.Append("\n}\n");

        // 写穿：File.WriteAllText 的内容可能滞留 OS 缓存，断电即丢。
        // FileStream + Flush(true) 强制内容与元数据落盘。
        using (var stream = new FileStream(tmp, FileMode.Create, FileAccess.Write, FileShare.None))
        {
            var bytes = Encoding.UTF8.GetBytes(sb.ToString());
            stream.Write(bytes, 0, bytes.Length);
            stream.Flush(true);
        }

        // File.Replace 是原子元数据操作：断电后 journal 要么是旧内容要么是
        // 新内容，不会出现"已删除、rename 未完成"的 journal 缺失窗口。
        if (File.Exists(path))
        {
            try { File.Replace(tmp, path, null); }
            catch (IOException)
            {
                // 非 NTFS 等不支持 Replace 的文件系统：回退 delete+move
                if (File.Exists(path)) File.Delete(path);
                File.Move(tmp, path);
            }
        }
        else
        {
            File.Move(tmp, path);
        }

        // 尽力刷新目录项使 rename/replace 持久；不可行时文件内容已写穿兜底
        FlushDirectoryEntries(Path.GetDirectoryName(path));
    }

    /// 尽力而为：刷新目录句柄让目录项（rename 结果）落盘。
    /// 打不开写句柄（权限不足/文件系统不支持）时静默跳过。
    private static void FlushDirectoryEntries(string directory)
    {
        try
        {
            const uint GenericWrite = 0x40000000;
            const uint FileFlagBackupSemantics = 0x02000000;
            IntPtr handle = CreateFile(directory, GenericWrite, 7, IntPtr.Zero, 3,
                FileFlagBackupSemantics, IntPtr.Zero);
            if (handle.ToInt64() == -1) return;
            try { FlushFileBuffers(handle); }
            finally { CloseHandle(handle); }
        }
        catch { }
    }

    private static void ClearUpdateJournal(string root)
    {
        try { File.Delete(UpdateJournalPath(root)); } catch { }
    }

    private static string ReadUpdateJournalRaw(string root)
    {
        try { return File.ReadAllText(UpdateJournalPath(root)); }
        catch { return ""; }
    }

    private static string ReadUpdateJournalState(string root)
    {
        return ExtractJsonField(ReadUpdateJournalRaw(root), "state") ?? "";
    }

    private static string ExtractJsonField(string json, string field)
    {
        var key = "\"" + field + "\"";
        int keyIdx = json.IndexOf(key, StringComparison.Ordinal);
        if (keyIdx < 0) return null;
        int colon = json.IndexOf(':', keyIdx + key.Length);
        if (colon < 0) return null;
        int open = json.IndexOf('"', colon);
        if (open < 0) return null;
        int close = json.IndexOf('"', open + 1);
        if (close < 0) return null;
        return json.Substring(open + 1, close - open - 1);
    }

    private static string Sha256OfFile(string path)
    {
        using (var sha = System.Security.Cryptography.SHA256.Create())
        using (var stream = File.OpenRead(path))
        {
            var hash = sha.ComputeHash(stream);
            var sb = new StringBuilder(hash.Length * 2);
            foreach (var b in hash) sb.Append(b.ToString("x2"));
            return sb.ToString();
        }
    }

    // --- 安全解压（P1：zip bomb 防御）-------------------------------------
    // 下载侧已限制 ZIP ≤ 300 MB（update::MAX_UPDATE_BYTES），但
    // ZipFile.ExtractToDirectory 不限制解压后总大小、entry 数量、单 entry 大小，
    // 也不拦截 entry 名路径穿越（../ 或绝对路径）。微软官方明确建议处理不完全
    // 可信 archive 时手动枚举 entry 并限制。ZIP 已经过签名 manifest + SHA256
    // 验签，这是 defense in depth：即便签名链被攻破或 staging 文件被替换，
    // 也不会因解压耗尽磁盘 / 覆盖系统文件。
    private const int MaxZipEntries = 10000;
    private const long MaxZipTotalUncompressed = 2L * 1024 * 1024 * 1024; // 2 GB
    private const long MaxZipSingleEntry = 1L * 1024 * 1024 * 1024;      // 1 GB

    private static void SafeExtractToDirectory(string zipPath, string destDir)
    {
        Directory.CreateDirectory(destDir);
        string destRoot = Path.GetFullPath(destDir);
        if (!destRoot.EndsWith(Path.DirectorySeparatorChar.ToString()))
            destRoot += Path.DirectorySeparatorChar;

        long totalUncompressed = 0;
        int entryCount = 0;

        using (var archive = System.IO.Compression.ZipFile.OpenRead(zipPath))
        {
            foreach (var entry in archive.Entries)
            {
                entryCount++;
                if (entryCount > MaxZipEntries)
                    throw new InvalidOperationException(
                        "ZIP entry count exceeds limit (" + MaxZipEntries + ")");

                long entrySize = entry.Length;
                if (entrySize > MaxZipSingleEntry)
                    throw new InvalidOperationException(
                        "ZIP entry '" + entry.FullName + "' exceeds single-entry size limit");
                totalUncompressed += entrySize;
                if (totalUncompressed > MaxZipTotalUncompressed)
                    throw new InvalidOperationException(
                        "ZIP uncompressed total size exceeds limit");

                // 路径穿越防御：解析 entry 目标绝对路径，必须在 destRoot 之下。
                // 空 entry 名（目录占位）跳过；含 `..` 或盘符根的 entry 拒绝。
                string entryName = entry.FullName;
                if (string.IsNullOrEmpty(entryName)) continue;
                // 标准化分隔符后再判断相对路径穿越
                string normName = entryName.Replace('/', Path.DirectorySeparatorChar);
                string destPath = Path.GetFullPath(Path.Combine(destDir, normName));
                if (!destPath.StartsWith(destRoot, StringComparison.OrdinalIgnoreCase))
                    throw new InvalidOperationException(
                        "ZIP entry '" + entry.FullName + "' escapes destination directory");

                // 目录 entry（以分隔符结尾）：创建目录跳过文件写入
                if (normName.EndsWith(Path.DirectorySeparatorChar.ToString()))
                {
                    Directory.CreateDirectory(destPath);
                    continue;
                }
                Directory.CreateDirectory(Path.GetDirectoryName(destPath));

                entry.ExtractToFile(destPath, overwrite: true);
            }
        }
    }

    /// App/ 是否可运行（含内层主程序）
    private static bool AppDirValid(string root)
    {
        return File.Exists(Path.Combine(root, "App", "ClashEdge", "ClashEdge.exe"));
    }

    /// 最新的 App.old-* 备份目录
    private static string NewestBackupDir(string root)
    {
        string backup = null;
        long best = -1;
        try
        {
            foreach (var dir in Directory.GetDirectories(root, "App.old-*"))
            {
                long ticks;
                var suffix = dir.Substring(dir.LastIndexOf('-') + 1);
                if (!long.TryParse(suffix, out ticks)) continue;
                if (ticks > best) { best = ticks; backup = dir; }
            }
        }
        catch { }
        return backup;
    }

    /// 残留的 App.new-* 临时目录
    private static string NewestTempAppDir(string root)
    {
        string temp = null;
        long best = -1;
        try
        {
            foreach (var dir in Directory.GetDirectories(root, "App.new-*"))
            {
                long ticks;
                var suffix = dir.Substring(dir.LastIndexOf('-') + 1);
                if (!long.TryParse(suffix, out ticks)) continue;
                if (ticks > best) { best = ticks; temp = dir; }
            }
        }
        catch { }
        return temp;
    }

    /// Launcher 自更新事务的恢复。必须在任何全局清理（特别是
    /// ClashEdge.exe.old-* 无条件删除）之前调用——根 exe 可能已改名走，
    /// .old-<txid> 是恢复依据，先删就永远无法启动了。
    /// 保证：本方法正常返回后根目录一定存在一个可启动的 launcher。
    private static void RecoverLauncherUpdate(string root, string state, bool silent)
    {
        var json = ReadUpdateJournalRaw(root);
        string oldPath = ExtractJsonField(json, "launcher_old");
        string newPath = ExtractJsonField(json, "launcher_new");
        string newSha = ExtractJsonField(json, "launcher_sha");
        string rootExe = Path.Combine(root, "ClashEdge.exe");

        if (state == "launcher-stage")
        {
            // 阶段1 后断电：旧 launcher 尚未移动，根 exe 完好，清 .new 暂存即可
        }
        else if (state == "launcher-swap-old" || state == "launcher-replace")
        {
            if (!File.Exists(rootExe))
            {
                // 根 exe 已改名走：优先用 SHA256 校验通过的 .new 恢复为新版；
                // .new 缺失/损坏时从 .old 恢复旧版。
                // 两个文件都不会在本事务中被提前删除，因此至少有一个可用。
                string promote = null;
                if (newPath != null && File.Exists(newPath))
                {
                    bool shaOk = true;
                    if (!string.IsNullOrEmpty(newSha))
                    {
                        try
                        {
                            shaOk = string.Equals(Sha256OfFile(newPath), newSha,
                                StringComparison.OrdinalIgnoreCase);
                        }
                        catch { shaOk = false; }
                    }
                    if (shaOk) promote = newPath;
                }
                if (promote == null && oldPath != null && File.Exists(oldPath))
                    promote = oldPath;
                if (promote == null)
                    throw new InvalidOperationException(
                        "launcher 恢复失败：根 exe 缺失且无可用备份");
                File.Move(promote, rootExe);
                if (!silent)
                    MessageBox.Show("检测到上次更新未完成，已自动恢复启动器。",
                        "更新恢复", MessageBoxButtons.OK, MessageBoxIcon.Information);
            }
            // 根 exe 存在：swap-old 尚未执行，或 swap-new 已完成——
            // 两种情况根 exe 都可启动，仅需清理残留
        }
        else if (state == "launcher-commit")
        {
            // 新 launcher 已 rename 到位：仅剩 .old 备份清理
        }

        // 清理 launcher 残留（journal 记录 + 通配兜底）
        try { if (oldPath != null && File.Exists(oldPath)) File.Delete(oldPath); } catch { }
        try { if (newPath != null && File.Exists(newPath)) File.Delete(newPath); } catch { }
        try
        {
            foreach (var f in Directory.GetFiles(root, "ClashEdge.exe.old-*"))
                File.Delete(f);
        }
        catch { }
        try
        {
            foreach (var f in Directory.GetFiles(root, "ClashEdge.exe.new-*"))
                File.Delete(f);
        }
        catch { }
    }

    /// P0：启动自愈——上次更新中断时先恢复到确定可运行状态。
    /// 必须在检查 ClashEdge.exe 是否存在之前执行（App/ 或根 launcher 可能被改名走）。
    private static void RecoverInterruptedUpdate(string root, bool silent)
    {
        string state = ReadUpdateJournalState(root);
        if (string.IsNullOrEmpty(state)) return;

        string appRoot = Path.Combine(root, "App");
        string staging = Path.Combine(root, "Data", "update-staging");
        string backup = NewestBackupDir(root);
        string tempApp = NewestTempAppDir(root);

        try
        {
            // P0：launcher 事务恢复必须先于 App 恢复与任何全局清理——
            // 后置的 ClashEdge.exe.old-* 无条件删除会销毁恢复依据。
            if (state.StartsWith("launcher-", StringComparison.Ordinal))
            {
                RecoverLauncherUpdate(root, state, silent);
            }
            else if (state == "swapping")
            {
                if (AppDirValid(root))
                {
                    // 交换已完成（或未开始——App 是旧版或新版都 valid）
                    if (backup != null) try { Directory.Delete(backup, true); } catch { }
                    if (tempApp != null) try { Directory.Delete(tempApp, true); } catch { }
                }
                else if (backup != null)
                {
                    // 交换中断：旧 App 已改名，新 App 未就位 → 从备份恢复
                    if (Directory.Exists(appRoot))
                        try { Directory.Delete(appRoot, true); } catch { }
                    Directory.Move(backup, appRoot);
                    if (tempApp != null) try { Directory.Delete(tempApp, true); } catch { }
                    if (!silent)
                        MessageBox.Show("检测到上次更新未完成，已自动恢复到更新前的版本。",
                            "更新恢复", MessageBoxButtons.OK, MessageBoxIcon.Information);
                }
                // 无备份且 App 无效：无物可恢复，后续 File.Exists 会给出明确报错
            }
            // pending / verified / committed：App 未被动过或已完成，无需处理

            try { Directory.Delete(staging, true); } catch { }
            try
            {
                foreach (var f in Directory.GetFiles(root, "ClashEdge.exe.old-*"))
                    File.Delete(f);
            }
            catch { }
            // launcher stage 阶段断电（journal 仍为 committed）遗留的 .new 暂存
            try
            {
                foreach (var f in Directory.GetFiles(root, "ClashEdge.exe.new-*"))
                    File.Delete(f);
            }
            catch { }
        }
        catch (Exception ex)
        {
            try
            {
                var logPath = Path.Combine(root, "Data", "launcher-error.log");
                Directory.CreateDirectory(Path.GetDirectoryName(logPath));
                File.AppendAllText(logPath,
                    "[" + DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss")
                    + "] update recovery failed (state=" + state + "): " + ex.Message
                    + Environment.NewLine);
            }
            catch { }
            return; // 保留 journal，下次启动再试
        }

        ClearUpdateJournal(root);
    }

    /// 应用暂存更新。事务结构（P0 修正）：
    ///   复验 → 解压 → 复制到 App.new-xxx → 验证 → 写 durable journal(swapping)
    ///   → 原子交换(旧→backup, new→App) → 验证 → journal(committed) → 删备份
    ///   → Launcher 自更新（最后）→ 清暂存
    private static void ApplyPendingUpdate(string root, bool silent)
    {
        string staging;
        try { staging = Path.Combine(root, "Data", "update-staging"); }
        catch { return; }
        try
        {
            var pendingPath = Path.Combine(staging, "pending.json");
            if (!File.Exists(pendingPath)) return;
            WriteUpdateJournal(root, "pending");

            var json = File.ReadAllText(pendingPath);
            string zipPath = ExtractJsonField(json, "zip_path");
            string expectedSha = ExtractJsonField(json, "sha256");
            string version = ExtractJsonField(json, "version") ?? "?";
            if (string.IsNullOrEmpty(zipPath) || string.IsNullOrEmpty(expectedSha)
                || !File.Exists(zipPath))
            {
                Directory.Delete(staging, true);
                ClearUpdateJournal(root);
                return;
            }

            string actualSha = Sha256OfFile(zipPath);
            if (!string.Equals(actualSha, expectedSha, StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("SHA256 mismatch on staged update");

            var extracted = Path.Combine(staging, "extracted");
            if (Directory.Exists(extracted)) Directory.Delete(extracted, true);
            SafeExtractToDirectory(zipPath, extracted);

            // 定位应用根
            string appRoot = null;
            if (File.Exists(Path.Combine(extracted, "App", "ClashEdge", "ClashEdge.exe")))
                appRoot = extracted;
            else
            {
                foreach (var dir in Directory.GetDirectories(extracted))
                {
                    if (File.Exists(Path.Combine(dir, "App", "ClashEdge", "ClashEdge.exe")))
                    {
                        appRoot = dir;
                        break;
                    }
                }
            }
            if (appRoot == null)
                throw new InvalidOperationException(
                    "Staged update does not contain App/ClashEdge/ClashEdge.exe");

            WriteUpdateJournal(root, "verified");

            // 复制到独立临时目录（不在 App/ 内逐文件复制——消除半成品风险）
            string txid = DateTime.Now.Ticks.ToString();
            string tempApp = Path.Combine(root, "App.new-" + txid);
            string backup = Path.Combine(root, "App.old-" + txid);
            string rootApp = Path.Combine(root, "App");

            CopyMissing(Path.Combine(appRoot, "App"), tempApp);
            if (!File.Exists(Path.Combine(tempApp, "ClashEdge", "ClashEdge.exe")))
            {
                try { Directory.Delete(tempApp, true); } catch { }
                throw new InvalidOperationException("New App/ failed pre-swap validation");
            }

            // 关键：durable journal 必须在破坏性交换之前写入。
            // 写失败 → 中止（删除临时目录），不触碰旧 App/。
            try { WriteUpdateJournal(root, "swapping"); }
            catch (Exception ex)
            {
                try { Directory.Delete(tempApp, true); } catch { }
                throw new InvalidOperationException("Cannot persist update journal; aborting before swap: " + ex.Message, ex);
            }

            // 原子交换：旧 App → backup，新 App → App/
            Directory.Move(rootApp, backup);
            try
            {
                Directory.Move(tempApp, rootApp);
            }
            catch
            {
                // 新 App 移入失败：恢复旧 App
                try { if (Directory.Exists(rootApp)) Directory.Delete(rootApp, true); } catch { }
                try { Directory.Move(backup, rootApp); } catch { }
                ClearUpdateJournal(root);
                throw;
            }
            if (!AppDirValid(root))
            {
                try { Directory.Delete(rootApp, true); } catch { }
                try { Directory.Move(backup, rootApp); } catch { }
                ClearUpdateJournal(root);
                throw new InvalidOperationException("New App/ failed post-swap validation");
            }

            WriteUpdateJournal(root, "committed");
            try { Directory.Delete(backup, true); } catch { }

            // Launcher 自更新放在 App commit 之后（避免混合版本状态）。
            // P0：四阶段断电安全事务——每个断电点之后根目录
            // 至少存在一个可启动的 launcher。
            try
            {
                ApplyLauncherSelfUpdate(root, appRoot, txid);
            }
            catch
            {
                // 尽力就地恢复；恢复失败必须传播到外层，保留 journal/staging
                // 供旧 Launcher 下次启动继续处理，不能伪装成更新成功。
                string launcherState = ReadUpdateJournalState(root);
                if (!launcherState.StartsWith("launcher-", StringComparison.Ordinal))
                    throw;
                RecoverLauncherUpdate(root, launcherState, true);
            }

            Directory.Delete(staging, true);
            ClearUpdateJournal(root);

            if (!silent)
                MessageBox.Show("ClashEdge 已更新到 " + version + "。", "更新完成",
                    MessageBoxButtons.OK, MessageBoxIcon.Information);
        }
        catch (Exception ex)
        {
            // 进入 swapping / launcher-* 后不清 journal——让下次启动恢复
            string state = ReadUpdateJournalState(root);
            if (state != "swapping"
                && !state.StartsWith("launcher-", StringComparison.Ordinal))
            {
                try { Directory.Delete(staging, true); } catch { }
                ClearUpdateJournal(root);
            }
            if (!silent)
                MessageBox.Show("更新应用失败，已保留当前版本。\n\n" + ex.Message,
                    "更新失败", MessageBoxButtons.OK, MessageBoxIcon.Warning);
        }
    }

    /// Launcher 自更新：先准备新文件，再用系统原子替换保持根入口始终存在。
    /// 替换失败时保留旧 Launcher 并由调用方保留 journal；不再执行
    /// “先把正在运行的根 exe 改名，再把新 exe 移入”的可启动性空窗。
    private static void ApplyLauncherSelfUpdate(string root, string appRoot, string txid)
    {
        string newLauncher = Path.Combine(appRoot, "ClashEdge.exe");
        string selfExe = System.Reflection.Assembly.GetExecutingAssembly().Location;
        if (!File.Exists(newLauncher) || string.IsNullOrEmpty(selfExe)
            || !File.Exists(selfExe)) return;
        if (string.Equals(Sha256OfFile(newLauncher), Sha256OfFile(selfExe),
            StringComparison.OrdinalIgnoreCase)) return;

        string stagedNew = Path.Combine(root, "ClashEdge.exe.new-" + txid);
        string stagedOld = selfExe + ".old-" + txid;

        // 阶段1 stage：复制新 launcher（复制失败不影响任何现有文件）
        string newSha;
        try
        {
            File.Copy(newLauncher, stagedNew, true);
            newSha = Sha256OfFile(stagedNew);
            WriteUpdateJournal(root, "launcher-stage", null, stagedNew, newSha);
        }
        catch
        {
            try { File.Delete(stagedNew); } catch { }
            return; // journal 仍是 committed：根 exe 完好，无物需恢复
        }

        // 原子替换：目标根文件不会先被移走。若系统不支持替换运行中的
        // executable，调用方会保留旧 Launcher 并清理/重试，而不会制造空窗。
        WriteUpdateJournal(root, "launcher-replace", stagedOld, stagedNew, newSha);
        File.Replace(stagedNew, selfExe, stagedOld, true);
        WriteUpdateJournal(root, "launcher-commit", stagedOld, null, null);

        // 阶段4 cleanup：删旧版备份（运行中进程可能锁定，失败留给下次启动清理）
        try { File.Delete(stagedOld); } catch { }
        ClearUpdateJournal(root);
    }

    // --- 故障注入测试 ---------------------------------------------------------
    // 模拟每个 kill 点的恢复行为，验证不会出现混合/缺失状态。
    // 由 ClashEdge.exe --test-recovery 触发运行。

    private static string TestCreateRoot(string tag)
    {
        var dir = Path.Combine(Path.GetTempPath(), "clashedge-launcher-test-" + tag + "-" + DateTime.Now.Ticks);
        Directory.CreateDirectory(dir);
        Directory.CreateDirectory(Path.Combine(dir, "Data"));
        var exePath = Path.Combine(dir, "App", "ClashEdge", "ClashEdge.exe");
        Directory.CreateDirectory(Path.GetDirectoryName(exePath));
        File.WriteAllText(exePath, "dummy");
        return dir;
    }

    private static string ReadText(string path)
    {
        try { return File.ReadAllText(path); }
        catch { return null; }
    }

    // The recovery checks historically only printed FAIL and still returned 0,
    // which let CI publish a broken launcher. Track assertion output so the
    // process exit code reflects every failed check without duplicating 48 calls.
    private sealed class FailureTrackingWriter : TextWriter
    {
        private readonly TextWriter inner;
        public int Failures { get; private set; }

        public FailureTrackingWriter(TextWriter inner) { this.inner = inner; }
        public override Encoding Encoding { get { return inner.Encoding; } }
        public override void Write(char value) { inner.Write(value); }
        public override void WriteLine(string value)
        {
            if (value != null && value.IndexOf("FAIL", StringComparison.Ordinal) >= 0)
                Failures++;
            inner.WriteLine(value);
        }
    }

    private static int RunRecoveryTests()
    {
        var originalOut = Console.Out;
        var trackingOut = new FailureTrackingWriter(originalOut);
        Console.SetOut(trackingOut);
        int total = 0;

        // 1. pending 状态
        {
            var root = TestCreateRoot("pending");
            Directory.CreateDirectory(Path.Combine(root, "Data", "update-staging"));
            File.WriteAllText(Path.Combine(root, "Data", "update-staging", "pending.json"), "{}");
            WriteUpdateJournal(root, "pending");
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". pending: " + (AppDirValid(root) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". pending cleaned: " + (!Directory.Exists(Path.Combine(root, "Data", "update-staging")) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". pending journal: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // 2. verified 状态
        {
            var root = TestCreateRoot("verified");
            WriteUpdateJournal(root, "verified");
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". verified: " + (AppDirValid(root) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". verified journal: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // 3. swapping + App 有效 + 备份存在
        {
            var root = TestCreateRoot("swapping-valid");
            var backup = Path.Combine(root, "App.old-" + DateTime.Now.Ticks);
            var backupExe = Path.Combine(backup, "ClashEdge", "ClashEdge.exe");
            // P1-2：必须先创建目录再写文件（旧实现顺序反了：先 WriteAllText
            // 再 CreateDirectory，DirectoryNotFoundException 让测试 6 在 CI 上恒失败）。
            Directory.CreateDirectory(Path.GetDirectoryName(backupExe));
            File.WriteAllText(backupExe, "old");
            WriteUpdateJournal(root, "swapping");
            RecoverInterruptedUpdate(root, silent: true);
            Console.WriteLine(++total + ". swapping-valid: " + (AppDirValid(root) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". swapping backup: " + (!Directory.Exists(backup) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". swapping journal: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // 4. swapping + App 无效 + 备份存在
        {
            var root = TestCreateRoot("swapping-invalid");
            Directory.Delete(Path.Combine(root, "App"), true);
            var backup = Path.Combine(root, "App.old-" + DateTime.Now.Ticks);
            var oldExe = Path.Combine(backup, "ClashEdge", "ClashEdge.exe");
            Directory.CreateDirectory(Path.GetDirectoryName(oldExe));
            File.WriteAllText(oldExe, "restored");
            WriteUpdateJournal(root, "swapping");
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". swapping-invalid app: " + (AppDirValid(root) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". swapping-invalid journal: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // 5. swapping + App 有效 + 无备份
        {
            var root = TestCreateRoot("swapping-nobackup");
            WriteUpdateJournal(root, "swapping");
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". swapping-nobackup: " + (AppDirValid(root) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". swapping-nobackup journal: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // 6. committed 状态
        {
            var root = TestCreateRoot("committed");
            WriteUpdateJournal(root, "committed");
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". committed: " + (AppDirValid(root) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". committed journal: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // 7. 无 journal 不崩溃
        {
            var root = TestCreateRoot("nojournal");
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". nojournal: " + (AppDirValid(root) ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // T-l1 阶段1 后断电：journal=launcher-stage，根 exe 完好，.new 残留存在
        {
            var root = TestCreateRoot("l1-stage");
            string rootExe = Path.Combine(root, "ClashEdge.exe");
            File.WriteAllText(rootExe, "old-launcher");
            string stagedNew = Path.Combine(root, "ClashEdge.exe.new-111");
            File.WriteAllText(stagedNew, "new-launcher");
            WriteUpdateJournal(root, "launcher-stage", null, stagedNew, Sha256OfFile(stagedNew));
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". T-l1 root exe exists: " + (File.Exists(rootExe) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l1 root content: " + (ReadText(rootExe) == "old-launcher" ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l1 new residue cleaned: " + (!File.Exists(stagedNew) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l1 journal cleared: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // T-l2 阶段2 后断电：journal=launcher-swap-old，根 exe 已改名走，
        // .old-* 存在，无 .new → 从 .old 恢复旧版
        {
            var root = TestCreateRoot("l2-swapold-no-new");
            string rootExe = Path.Combine(root, "ClashEdge.exe");
            string stagedOld = Path.Combine(root, "ClashEdge.exe.old-222");
            File.WriteAllText(stagedOld, "old-launcher");
            WriteUpdateJournal(root, "launcher-swap-old", stagedOld,
                Path.Combine(root, "ClashEdge.exe.new-222"), null);
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". T-l2 root exe exists: " + (File.Exists(rootExe) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l2 root content (old restored): " + (ReadText(rootExe) == "old-launcher" ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l2 old consumed: " + (!File.Exists(stagedOld) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l2 journal cleared: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // T-l3 阶段2 后断电且 .new 已复制：根 exe 不存在，.old 与 .new 都存在
        // → .new SHA 校验通过，恢复为新版
        {
            var root = TestCreateRoot("l3-swapold-both");
            string rootExe = Path.Combine(root, "ClashEdge.exe");
            string stagedOld = Path.Combine(root, "ClashEdge.exe.old-333");
            string stagedNew = Path.Combine(root, "ClashEdge.exe.new-333");
            File.WriteAllText(stagedOld, "old-launcher");
            File.WriteAllText(stagedNew, "new-launcher");
            WriteUpdateJournal(root, "launcher-swap-old", stagedOld, stagedNew, Sha256OfFile(stagedNew));
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". T-l3 root exe exists: " + (File.Exists(rootExe) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l3 root content (new): " + (ReadText(rootExe) == "new-launcher" ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l3 old cleaned: " + (!File.Exists(stagedOld) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l3 journal cleared: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // T-l4 复制中断：.new 截断损坏，journal=launcher-swap-old 且根 exe 仍在
        // → 根 exe 原样保留，残留清理
        {
            var root = TestCreateRoot("l4-truncated-new");
            string rootExe = Path.Combine(root, "ClashEdge.exe");
            File.WriteAllText(rootExe, "old-launcher");
            string stagedOld = Path.Combine(root, "ClashEdge.exe.old-444");
            string stagedNew = Path.Combine(root, "ClashEdge.exe.new-444");
            File.WriteAllText(stagedOld, "old-launcher");
            File.WriteAllText(stagedNew, "new"); // 截断
            WriteUpdateJournal(root, "launcher-swap-old", stagedOld, stagedNew,
                Sha256OfFile(stagedNew)); // journal 记录的是损坏后的实际摘要
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". T-l4 root exe exists: " + (File.Exists(rootExe) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l4 root content unchanged: " + (ReadText(rootExe) == "old-launcher" ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l4 new residue cleaned: " + (!File.Exists(stagedNew) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l4 journal cleared: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // T-l5 复制后断电：根 exe 已是新版（swap-new 完成但 journal 未推进），
        // journal=launcher-swap-old，.old 存在 → 根 exe 保留新版，.old 清理
        {
            var root = TestCreateRoot("l5-swapnew-done");
            string rootExe = Path.Combine(root, "ClashEdge.exe");
            File.WriteAllText(rootExe, "new-launcher");
            string stagedOld = Path.Combine(root, "ClashEdge.exe.old-555");
            string stagedNew = Path.Combine(root, "ClashEdge.exe.new-555");
            File.WriteAllText(stagedOld, "old-launcher");
            WriteUpdateJournal(root, "launcher-swap-old", stagedOld, stagedNew, null);
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". T-l5 root exe exists: " + (File.Exists(rootExe) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l5 root content (new kept): " + (ReadText(rootExe) == "new-launcher" ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l5 old cleaned: " + (!File.Exists(stagedOld) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l5 journal cleared: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // T-l6 清理前断电：journal=launcher-commit，根 exe=新版，
        // .old 与 .new 残留 → 仅根 exe 保留，残留清理，journal 清空
        {
            var root = TestCreateRoot("l6-commit");
            string rootExe = Path.Combine(root, "ClashEdge.exe");
            File.WriteAllText(rootExe, "new-launcher");
            string stagedOld = Path.Combine(root, "ClashEdge.exe.old-666");
            string stagedNew = Path.Combine(root, "ClashEdge.exe.new-666");
            File.WriteAllText(stagedOld, "old-launcher");
            File.WriteAllText(stagedNew, "garbage");
            WriteUpdateJournal(root, "launcher-commit", stagedOld, null, null);
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". T-l6 root exe exists: " + (File.Exists(rootExe) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l6 root content (new): " + (ReadText(rootExe) == "new-launcher" ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l6 old cleaned: " + (!File.Exists(stagedOld) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l6 new residue cleaned: " + (!File.Exists(stagedNew) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l6 journal cleared: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // T-l7 组合：App 交换已 committed + launcher 处于 swap-old（根 exe 已改名走）
        // → 一次性恢复后 App 与根 exe 都有效
        {
            var root = TestCreateRoot("l7-app-committed");
            string rootExe = Path.Combine(root, "ClashEdge.exe");
            string stagedOld = Path.Combine(root, "ClashEdge.exe.old-777");
            File.WriteAllText(stagedOld, "old-launcher");
            WriteUpdateJournal(root, "launcher-swap-old", stagedOld,
                Path.Combine(root, "ClashEdge.exe.new-777"), null);
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". T-l7 app valid: " + (AppDirValid(root) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l7 root exe exists: " + (File.Exists(rootExe) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l7 root content: " + (ReadText(rootExe) == "old-launcher" ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l7 journal cleared: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        // T-l8 附加：根 exe 缺失且 .new 损坏（SHA 不符）→ 拒绝升级到损坏文件，
        // 从 .old 恢复旧版
        {
            var root = TestCreateRoot("l8-corrupt-new");
            string rootExe = Path.Combine(root, "ClashEdge.exe");
            string stagedOld = Path.Combine(root, "ClashEdge.exe.old-888");
            string stagedNew = Path.Combine(root, "ClashEdge.exe.new-888");
            File.WriteAllText(stagedOld, "old-launcher");
            File.WriteAllText(stagedNew, "corrupt");
            WriteUpdateJournal(root, "launcher-swap-old", stagedOld, stagedNew,
                Sha256OfFile(Path.Combine(root, "App", "ClashEdge", "ClashEdge.exe"))); // 与 .new 不符
            RecoverInterruptedUpdate(root, true);
            Console.WriteLine(++total + ". T-l8 root exe exists: " + (File.Exists(rootExe) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l8 root content (old fallback): " + (ReadText(rootExe) == "old-launcher" ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l8 corrupt new cleaned: " + (!File.Exists(stagedNew) ? "PASS" : "FAIL"));
            Console.WriteLine(++total + ". T-l8 journal cleared: " + (ReadUpdateJournalState(root) == "" ? "PASS" : "FAIL"));
            Directory.Delete(root, true);
        }

        int exitCode = trackingOut.Failures == 0 ? 0 : 1;
        Console.SetOut(originalOut);
        originalOut.WriteLine("\n" + total + " assertions checked; failures: " + trackingOut.Failures + ".");
        return exitCode;
    }

    [STAThread]
    private static int Main(string[] args)
    {
        // 故障注入测试模式
        if (args.Any(a => a == "--test-recovery"))
        {
            try { return RunRecoveryTests(); }
            catch (Exception ex) { Console.WriteLine("TEST ERROR: " + ex.Message); return 1; }
        }

        bool silent = args.Any(a => a == "--clash-edge-autostart");
        try
        {
            var root = AppDomain.CurrentDomain.BaseDirectory.TrimEnd(Path.DirectorySeparatorChar);

            // P0：先恢复中断的更新——App/ 可能被改名走，ClashEdge.exe 可能不存在。
            // 必须在检查 executable 存在性之前执行，否则 old_renamed 状态下会直接报错退出。
            RecoverInterruptedUpdate(root, silent);

            var appDirectory = Path.Combine(root, "App", "ClashEdge");
            var executable = Path.Combine(appDirectory, "ClashEdge.exe");
            if (!File.Exists(executable)) throw new FileNotFoundException("找不到 ClashEdge 主程序。请完整解压后再启动。", executable);

            var data = Path.Combine(root, "Data");
            CopyMissing(Path.Combine(root, "App", "DefaultData"), data);
            Directory.CreateDirectory(data);
            EnsureDataJunction(Path.Combine(appDirectory, "data"), data);

            // 恢复完成后应用新暂存更新
            ApplyPendingUpdate(root, silent);

            var home = Path.Combine(data, "Home");
            Directory.CreateDirectory(home);
            var forwarded = string.Join(" ", args.Select(Quote));
            var start = new ProcessStartInfo(executable, "--user-data-dir=" + Quote(Path.Combine(data, "SessionData")) + " " + forwarded)
            {
                WorkingDirectory = appDirectory,
                UseShellExecute = false,
                CreateNoWindow = true,
            };
            start.EnvironmentVariables["CLASH_EDGE_DATA_DIR"] = data;
            start.EnvironmentVariables["HOME"] = home;
            start.EnvironmentVariables["APPDATA"] = Path.Combine(data, "AppData");
            start.EnvironmentVariables["LOCALAPPDATA"] = Path.Combine(data, "LocalAppData");
            if (Process.Start(start) == null) throw new InvalidOperationException("主程序未能启动。");
            return 0;
        }
        catch (Exception error)
        {
            if (silent)
            {
                try
                {
                    var root = AppDomain.CurrentDomain.BaseDirectory.TrimEnd(Path.DirectorySeparatorChar);
                    var logPath = Path.Combine(root, "Data", "launcher-error.log");
                    Directory.CreateDirectory(Path.GetDirectoryName(logPath));
                    File.AppendAllText(logPath,
                        "[" + DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss") + "] " + error + Environment.NewLine);
                }
                catch { }
                return 1;
            }
            MessageBox.Show(error.Message, "ClashEdge", MessageBoxButtons.OK, MessageBoxIcon.Error);
            return 1;
        }
    }
}

using System;
using System.Diagnostics;
using System.IO;
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
    private static extern bool DeviceIoControl(
        IntPtr hDevice,
        int dwIoControlCode,
        IntPtr lpInBuffer,
        int nInBufferSize,
        byte[] lpOutBuffer,
        int nOutBufferSize,
        out int lpBytesReturned,
        IntPtr lpOverlapped);

    /// 解析目录联接（junction）的替代名称，形如 \??\C:\real\path。
    /// 不是挂载点联接、打开失败或解析失败时返回 null。
    private static string GetJunctionTarget(string path)
    {
        // 以零访问权打开 reparse point 本身（FILE_FLAG_OPEN_REPARSE_POINT
        // 阻止内核穿透到目标；零访问权避免对目标目录的任何权限要求）。
        IntPtr handle = CreateFile(path, 0, 7, IntPtr.Zero, 3,
            0x02000000 /* FILE_FLAG_BACKUP_SEMANTICS */
                | 0x00200000 /* FILE_FLAG_OPEN_REPARSE_POINT */,
            IntPtr.Zero);
        if (handle.ToInt64() == -1) return null;
        try
        {
            var buffer = new byte[16 * 1024];
            int returned;
            if (!DeviceIoControl(handle, FsctlGetReparsePoint, IntPtr.Zero, 0,
                    buffer, buffer.Length, out returned, IntPtr.Zero))
                return null;

            // REPARSE_DATA_BUFFER：Tag(4) DataLength(2) Reserved(2)
            // MountPoint：SubstituteNameOffset(2) SubstituteNameLength(2)
            //             PrintNameOffset(2) PrintNameLength(2) PathBuffer...
            uint tag = BitConverter.ToUInt32(buffer, 0);
            if (tag != IoReparseTagMountPoint) return null;
            ushort substituteOffset = BitConverter.ToUInt16(buffer, 8);
            ushort substituteLength = BitConverter.ToUInt16(buffer, 10);
            int start = 16 + substituteOffset;
            if (start + substituteLength > returned) return null;
            return Encoding.Unicode.GetString(buffer, start, substituteLength);
        }
        finally
        {
            CloseHandle(handle);
        }
    }

    /// 规范化联接目标路径用于比对：剥离 \??\ / \\?\ 前缀并取完整路径。
    private static string NormalizeJunctionPath(string path)
    {
        if (path == null) return null;
        if (path.StartsWith(@"\??\", StringComparison.Ordinal)) path = path.Substring(4);
        else if (path.StartsWith(@"\\?\", StringComparison.Ordinal)) path = path.Substring(4);
        try { return Path.GetFullPath(path).TrimEnd(Path.DirectorySeparatorChar); }
        catch { return path.TrimEnd(Path.DirectorySeparatorChar); }
    }

    /// P1-6：校验已存在的联接真实指向便携根下的 Data 目录。
    /// 目标不符或解析失败时抛异常终止启动——防止 junction 被篡改后
    /// 应用把用户数据写到任意位置（如系统目录或其他盘）。
    private static void ValidateJunctionTarget(string appDataDirectory, string portableDataDirectory)
    {
        string actual = NormalizeJunctionPath(GetJunctionTarget(appDataDirectory));
        string expected = NormalizeJunctionPath(portableDataDirectory);
        bool match = actual != null && string.Equals(actual, expected, StringComparison.OrdinalIgnoreCase);
        if (!match)
        {
            throw new InvalidOperationException(
                "数据目录联接校验失败（App\\ClashEdge\\data 未指向本便携包的 Data 目录）。" +
                "\n预期目标: " + (expected ?? "<未知>") +
                "\n实际目标: " + (actual ?? "<解析失败>") +
                "\n为防数据写入错误位置已拒绝启动。请删除 App\\ClashEdge\\data 后重新运行，或重新解压完整便携包。");
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

    /// 判断路径是否存在（含损坏的联接——损坏的 reparse point 上 Directory.Exists
    /// 会返回 false，但路径仍被 mklink 占用，必须先删掉才能重建）。
    private static bool PathExists(string path)
    {
        try { File.GetAttributes(path); return true; }
        catch { return false; }
    }

    /// 把真实目录的内容并入目标（不覆盖目标已有文件），然后整体删除源目录。
    /// 用于把"手动/残留创建的真实 data 目录"收编进 Data/ 后重建联接。
    private static void MoveContents(string sourceDir, string destinationDir)
    {
        Directory.CreateDirectory(destinationDir);
        foreach (var file in Directory.GetFiles(sourceDir, "*", SearchOption.AllDirectories))
        {
            var target = file.Replace(sourceDir, destinationDir);
            Directory.CreateDirectory(Path.GetDirectoryName(target));
            if (File.Exists(target))
                File.Delete(file); // 目标已有同名文件，丢弃残留副本
            else
                File.Move(file, target);
        }
        // 删除迁移后遗留的空目录
        foreach (var dir in Directory.GetDirectories(sourceDir, "*", SearchOption.AllDirectories)
                     .OrderByDescending(d => d.Length))
        {
            if (Directory.GetFiles(dir, "*", SearchOption.AllDirectories).Length == 0)
                try { Directory.Delete(dir, true); } catch { }
        }
    }

    /// 确保 App/ClashEdge/data 是指向便携根 Data/ 的有效联接。四种情况：
    /// 1. 已是联接且目标正确 → 校验通过，直接返回（P1-6）；
    /// 2. 是联接但目标不符 / 解析失败 → 报错退出（防篡改）；
    /// 3. 是真实目录（手动创建或旧版本残留）→ 内容并入 Data/ 后删除并重建联接；
    /// 4. 是损坏的联接（目标不存在）→ 删除后重建。
    private static void EnsureDataJunction(string appDataDirectory, string portableDataDirectory)
    {
        if (PathExists(appDataDirectory))
        {
            var attrs = File.GetAttributes(appDataDirectory);
            if ((attrs & FileAttributes.ReparsePoint) != 0)
            {
                // 有效联接（目标存在）必须校验其真实目标；损坏联接
                // Directory.Exists == false，走删除重建。
                if (Directory.Exists(appDataDirectory))
                {
                    ValidateJunctionTarget(appDataDirectory, portableDataDirectory);
                    return;
                }
                File.Delete(appDataDirectory); // 只删联接本身，不递归目标
            }
            else
            {
                // 真实目录残留：并入 Data/ 后删除，重建联接。
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

    // --- Portable Updater apply (0.8.10 Phase 3) ------------------------------

    /// 从 pending.json 提取简单字符串字段（避免引入 JSON 依赖；C# 5 / 最小引用）
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

    /// 应用暂存更新：Data/update-staging/pending.json 存在时，
    /// 复验 ZIP 哈希 → 解压 → 结构校验（必须含 ClashEdge.exe）→
    /// 替换 App/ClashEdge（旧目录先改名保留，失败即回滚）→ 清理暂存。
    /// 任何失败都不阻断正常启动——保留旧版本继续运行，仅清理无效暂存。
    private static void ApplyPendingUpdate(string root, bool silent)
    {
        string staging;
        try { staging = Path.Combine(root, "Data", "update-staging"); }
        catch { return; }
        try
        {
            var pendingPath = Path.Combine(staging, "pending.json");
            if (!File.Exists(pendingPath)) return;

            var json = File.ReadAllText(pendingPath);
            string zipPath = ExtractJsonField(json, "zip_path");
            string expectedSha = ExtractJsonField(json, "sha256");
            string version = ExtractJsonField(json, "version") ?? "?";
            if (string.IsNullOrEmpty(zipPath) || string.IsNullOrEmpty(expectedSha)
                || !File.Exists(zipPath))
            {
                Directory.Delete(staging, true);
                return;
            }

            // 复验哈希（下载端已验一次；应用前不信任暂存区状态）
            string actualSha = Sha256OfFile(zipPath);
            if (!string.Equals(actualSha, expectedSha, StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("SHA256 mismatch on staged update");

            // 解压到临时目录
            var extracted = Path.Combine(staging, "extracted");
            if (Directory.Exists(extracted)) Directory.Delete(extracted, true);
            System.IO.Compression.ZipFile.ExtractToDirectory(zipPath, extracted);

            // 定位应用根：ZIP 根直接是应用，或带一层顶层目录
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

            // 启动器自更新：ZIP 根若带新版 ClashEdge.exe 且内容不同，
            // 用 rename 技巧换掉正在运行的自身（Windows 允许重命名运行中的映像）
            try
            {
                string newLauncher = Path.Combine(appRoot, "ClashEdge.exe");
                string selfExe = System.Reflection.Assembly.GetExecutingAssembly().Location;
                if (File.Exists(newLauncher) && File.Exists(selfExe)
                    && !string.Equals(Sha256OfFile(newLauncher), Sha256OfFile(selfExe),
                        StringComparison.OrdinalIgnoreCase))
                {
                    var selfOld = selfExe + ".old-" + DateTime.Now.Ticks;
                    File.Move(selfExe, selfOld);
                    File.Copy(newLauncher, selfExe, true);
                }
            }
            catch { } // 自更新失败不阻断 App/ 更新

            // 完整便携布局替换：以 appRoot 为新根，替换根下的 App/；
            // Data/ 在根级、不在包内，天然不受影响。
            string rootApp = Path.Combine(root, "App");
            string backup = rootApp + ".old-" + DateTime.Now.Ticks;
            Directory.Move(rootApp, backup);
            try
            {
                CopyMissing(Path.Combine(appRoot, "App"), rootApp);
            }
            catch
            {
                // 回滚：新内容复制失败 → 恢复旧 App/
                try { if (Directory.Exists(rootApp)) Directory.Delete(rootApp, true); } catch { }
                Directory.Move(backup, rootApp);
                throw;
            }
            try { Directory.Delete(backup, true); } catch { }
            Directory.Delete(staging, true);

            if (!silent)
                MessageBox.Show("ClashEdge 已更新到 " + version + "。", "更新完成",
                    MessageBoxButtons.OK, MessageBoxIcon.Information);
        }
        catch (Exception ex)
        {
            // 更新失败绝不阻断启动：清理暂存，继续用当前版本
            try { Directory.Delete(staging, true); } catch { }
            if (!silent)
                MessageBox.Show("更新应用失败，已保留当前版本。\n\n" + ex.Message,
                    "更新失败", MessageBoxButtons.OK, MessageBoxIcon.Warning);
        }
    }

    [STAThread]
    private static int Main(string[] args)
    {
        bool silent = args.Any(a => a == "--clash-edge-autostart");
        try
        {
            var root = AppDomain.CurrentDomain.BaseDirectory.TrimEnd(Path.DirectorySeparatorChar);
            var appDirectory = Path.Combine(root, "App", "ClashEdge");
            var executable = Path.Combine(appDirectory, "ClashEdge.exe");
            if (!File.Exists(executable)) throw new FileNotFoundException("找不到 ClashEdge 主程序。请完整解压后再启动。", executable);

            var data = Path.Combine(root, "Data");
            CopyMissing(Path.Combine(root, "App", "DefaultData"), data);
            Directory.CreateDirectory(data);
            EnsureDataJunction(Path.Combine(appDirectory, "data"), data);
            // 0.8.10 Portable Updater：拉起内层前应用已验签的暂存更新
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
            // 开机自启阶段不能弹框（会卡死登录），改为写日志文件静默失败。
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

using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Windows.Forms;

internal static class ClashEdgeLauncher
{
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

    /// 确保 App/ClashEdge/data 是有效联接。三种情况：
    /// 1. 已是有效联接 → 直接返回；
    /// 2. 是真实目录（手动创建或旧版本残留）→ 内容并入 Data/ 后删除并重建联接；
    /// 3. 是损坏的联接 → 删除后重建。
    /// 不再对真实目录报错，而是自愈。
    private static void EnsureDataJunction(string appDataDirectory, string portableDataDirectory)
    {
        if (PathExists(appDataDirectory))
        {
            var attrs = File.GetAttributes(appDataDirectory);
            if ((attrs & FileAttributes.ReparsePoint) != 0)
            {
                // 有效联接（目标存在）即返回；损坏联接 Directory.Exists == false，走重建。
                if (Directory.Exists(appDataDirectory)) return;
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

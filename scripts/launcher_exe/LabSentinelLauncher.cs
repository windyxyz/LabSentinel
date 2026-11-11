// LabSentinel 便携包启动 EXE（任务55）：无控制台入口，双击体验像"软件"而不是脚本。
//
// 编译（见 scripts/build_launcher_exe.py，用 .NET Framework 自带 csc，C# 5 语法）：
//   csc /nologo /target:winexe /win32manifest:app.manifest /r:System.Windows.Forms.dll /r:System.dll LabSentinelLauncher.cs
//
// 2026-11-11 反误报（AV）治理：无签名、无版本资源、无清单的裸 winexe 拉起解释器
// 执行用户目录脚本，是杀软启发式（HEUR:Trojan.Win32.Agent 类）的典型画像——卡巴斯基
// 实测按此处置了整份便携包 runtime。程序集属性（产品/版本/公司）进入托管元数据、
// /win32manifest 注入原生清单（asInvoker + 应用名描述），让 exe 具备可核验的正当身份；
// 配合启动链自身的干净审计（无 -c 动态执行 / 无 runpy / urlopen 仅回环健康检查，
// 见 docs/杀毒软件误报说明.md）。证书签名到位前，这是仓库内能做的最大减面项。
//
// 职责刻意保持最小：单实例/端口/健康检查/开浏览器全部由 Python 启动器（portable_launcher.py）
// 负责——EXE 只是把"双击 pythonw 脚本"包成无黑框的应用程序入口，不重复实现任何逻辑。
//   默认：启动 runtime\pythonw.exe scripts\portable_launcher.py 后自身退出（服务由 Python 托管）
//   --stop：启动 runtime\python.exe scripts\portable_launcher.py --stop，等待并返回其退出码
using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Windows.Forms;

[assembly: AssemblyTitle("LabSentinel")]
[assembly: AssemblyDescription("LabSentinel 便携版启动器（本地实验室安全监控）")]
[assembly: AssemblyProduct("LabSentinel")]
[assembly: AssemblyCompany("LabSentinel Project")]
[assembly: AssemblyCopyright("AGPL-3.0")]
[assembly: AssemblyVersion("0.2.0.0")]
[assembly: AssemblyFileVersion("0.2.0.0")]

namespace LabSentinelLauncher
{
    internal static class Program
    {
        private static string LogsDir()
        {
            return Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "LabSentinel", "logs");
        }

        [STAThread]
        private static int Main(string[] args)
        {
            try
            {
                string root = AppDomain.CurrentDomain.BaseDirectory;
                if (!root.EndsWith(Path.DirectorySeparatorChar.ToString(), StringComparison.Ordinal))
                {
                    root += Path.DirectorySeparatorChar;
                }
                string pythonw = Path.Combine(root, "runtime", "pythonw.exe");
                string python = Path.Combine(root, "runtime", "python.exe");
                string launcher = Path.Combine(root, "scripts", "portable_launcher.py");

                if (!File.Exists(pythonw) || !File.Exists(launcher))
                {
                    MessageBox.Show(
                        "找不到便携包组件：\n  " + pythonw + "\n  " + launcher +
                        "\n\n请确认 LabSentinel.exe 位于便携包根目录（与 启动.bat 同级）。\n" +
                        "日志目录：" + LogsDir(),
                        "LabSentinel 启动失败", MessageBoxButtons.OK, MessageBoxIcon.Error);
                    return 2;
                }

                bool stop = args.Length > 0 &&
                    string.Equals(args[0], "--stop", StringComparison.OrdinalIgnoreCase);

                // 2026-11-11 冒烟修复（R2-004 本地实机验证发现）：launcher 路径改为
                // 绝对路径 + 显式 WorkingDirectory。旧实现用相对路径 runpy.run_path(
                // 'scripts/portable_launcher.py') 并假设子进程继承 cwd=包根——双击
                // 时成立，但以其他 cwd 启动 exe（命令行/计划任务/冒烟脚本）时 runpy
                // 会解析到错误目录的 launcher，APP_ROOT 随之漂移（冒烟实录：找成了
                // 开发机仓库根的 runtime）。launcher 路径来自 exe 自身 BaseDirectory
                // 派生（非外部输入），引号拼接是可控的；停止信号仍为常量环境变量
                // LS_PORTABLE_STOP。
                ProcessStartInfo psi = new ProcessStartInfo();
                if (stop)
                {
                    psi.FileName = python;
                    psi.EnvironmentVariables["LS_PORTABLE_STOP"] = "1";
                }
                else
                {
                    psi.FileName = pythonw;
                }
                psi.Arguments = "\"" + launcher + "\"";
                psi.WorkingDirectory = root;
                psi.UseShellExecute = false;
                psi.CreateNoWindow = true;

                using (Process p = new Process { StartInfo = psi })
                {
                    p.Start();
                    if (stop)
                    {
                        p.WaitForExit();
                        return p.ExitCode;
                    }
                }
                return 0;  // 启动路径：Python 启动器接管（拉服务/等就绪/开浏览器），本进程即退
            }
            catch (Exception ex)
            {
                MessageBox.Show("启动器异常：" + ex.Message + "\n日志目录：" + LogsDir(),
                    "LabSentinel 启动失败", MessageBoxButtons.OK, MessageBoxIcon.Error);
                return 1;
            }
        }
    }
}

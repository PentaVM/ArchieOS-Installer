// ArchieOS for Windows in one file: the launcher, its engine, its texts and the signed release
// manifest travel inside this exe, which is what the downloads page offers.
//
//   ArchieOS-Setup.exe                 the first time: installs the launcher for this Windows user
//                                      (Install-ArchieOS-Launcher.ps1: %LOCALAPPDATA%\ArchieOS\Launcher,
//                                      the shortcuts, Settings → Apps) and opens it on Install.
//                                      After that: opens the launcher.
//   ArchieOS-Setup.exe -Autostart      the arguments go to the launcher (-Autostart, -Page modelo)
//   ArchieOS-Setup.exe --install       installs (or repairs) with no window and opens nothing;
//                                      exit code 0 or 1, the details in %TEMP%\ArchieOS-Setup.log
//   ArchieOS-Setup.exe --extract DIR   writes the files it carries to DIR and exits
//
// The installed copy is this same exe, as ArchieOS.exe in the launcher's folder; the desktop,
// Start menu and sign-in shortcuts point to it. A different build (a newer download) installs
// itself over the old one; a missing file is put back the same way. No administrator rights:
// everything stays in this user's profile.
//
// C# 5 and .NET Framework 4, so the compiler every Windows 10 and 11 carries builds it:
// Build-ArchieOS-Setup.ps1 (Windows) or build.sh (Linux), beside this file.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

[assembly: AssemblyTitle("ArchieOS")]
[assembly: AssemblyDescription("ArchieOS: installs and opens the launcher")]
[assembly: AssemblyCompany("ArchieOS")]
[assembly: AssemblyProduct("ArchieOS")]
[assembly: AssemblyCopyright("PentaVM, Xavi and Sheriff. Apache-2.0")]
[assembly: AssemblyVersion("0.1.0.0")]
[assembly: AssemblyFileVersion("0.1.0.0")]
[assembly: AssemblyInformationalVersion("0.1.0")]

namespace ArchieOS
{
    static class Setup
    {
        // Embedded files are named payload/<file>: Install-ArchieOS-Launcher.ps1 and what it copies.
        const string Payload = "payload/";
        const string InstallScript = "Install-ArchieOS-Launcher.ps1";

        static readonly string LogFile = Path.Combine(Path.GetTempPath(), "ArchieOS-Setup.log");

        [STAThread]
        static int Main(string[] args)
        {
            try
            {
                if (args.Length == 2 && args[0] == "--extract")
                {
                    Extract(args[1]);
                    return 0;
                }
                return Run(args);
            }
            catch (Exception e)
            {
                Log("error: " + e);
                // --install and --extract run from scripts: no window, the exit code says it.
                if (args.Length == 0 || !args[0].StartsWith("--", StringComparison.Ordinal)) ShowError(e.Message);
                return 1;
            }
        }

        static int Run(string[] args)
        {
            string self = Assembly.GetExecutingAssembly().Location;
            if (args.Length == 1 && args[0] == "--install")
            {
                string failure;
                return Install(self, out failure) ? 0 : 1;
            }
            string target = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"ArchieOS\Launcher");
            bool firstInstall = !File.Exists(Path.Combine(target, "ArchieOS-Launcher.ps1"));
            if (!IsInstalled(self, target))
            {
                // Two clicks in a row install once: the second one leaves the first to it.
                bool owner;
                using (Mutex busy = new Mutex(true, @"Local\ArchieOS-Setup", out owner))
                {
                    if (!owner) return 0;
                    if (!InstallWithProgress(self)) return 1;
                }
                // A new install opens on Install, which checks this PC and sets ArchieOS up.
                if (firstInstall && args.Length == 0) args = new string[] { "-Page", "instalar" };
            }
            OpenLauncher(target, args);
            return 0;
        }

        // Installed means this very build is the launcher's ArchieOS.exe and every file it carries
        // is in place (the installer script itself is the only one that stays out).
        static bool IsInstalled(string self, string target)
        {
            string installed = Path.Combine(target, "ArchieOS.exe");
            if (!File.Exists(installed)) return false;
            foreach (string name in PayloadNames())
            {
                if (name == InstallScript) continue;
                if (!File.Exists(Path.Combine(target, name))) return false;
            }
            if (string.Equals(Path.GetFullPath(self), Path.GetFullPath(installed), StringComparison.OrdinalIgnoreCase)) return true;
            return SameBytes(self, installed);
        }

        static bool SameBytes(string a, string b)
        {
            FileInfo fa = new FileInfo(a), fb = new FileInfo(b);
            if (fa.Length != fb.Length) return false;
            byte[] x = File.ReadAllBytes(a), y = File.ReadAllBytes(b);
            for (int i = 0; i < x.Length; i++) if (x[i] != y[i]) return false;
            return true;
        }

        static List<string> PayloadNames()
        {
            List<string> names = new List<string>();
            foreach (string resource in Assembly.GetExecutingAssembly().GetManifestResourceNames())
                if (resource.StartsWith(Payload, StringComparison.Ordinal)) names.Add(resource.Substring(Payload.Length));
            names.Sort(StringComparer.Ordinal);
            return names;
        }

        static void Extract(string dir)
        {
            Directory.CreateDirectory(dir);
            Assembly me = Assembly.GetExecutingAssembly();
            foreach (string name in PayloadNames())
            {
                if (name.IndexOfAny(new char[] { '/', '\\', ':' }) >= 0 || name.StartsWith(".", StringComparison.Ordinal))
                    throw new InvalidDataException("Bad embedded file name: " + name);
                using (Stream source = me.GetManifestResourceStream(Payload + name))
                using (FileStream file = File.Create(Path.Combine(dir, name)))
                    source.CopyTo(file);
            }
        }

        // The window says what is happening while Install-ArchieOS-Launcher.ps1 runs (a few seconds).
        static bool InstallWithProgress(string self)
        {
            Application.EnableVisualStyles();
            Form form = new Form();
            form.Text = "ArchieOS";
            form.FormBorderStyle = FormBorderStyle.FixedDialog;
            form.MaximizeBox = false;
            form.MinimizeBox = false;
            form.AutoScaleDimensions = new SizeF(96F, 96F);
            form.AutoScaleMode = AutoScaleMode.Dpi;
            form.StartPosition = FormStartPosition.CenterScreen;
            form.ClientSize = new Size(380, 96);
            try { form.Icon = Icon.ExtractAssociatedIcon(self); } catch (Exception) { }
            Label label = new Label();
            label.Text = Text("installing");
            label.SetBounds(20, 18, 340, 24);
            ProgressBar bar = new ProgressBar();
            bar.Style = ProgressBarStyle.Marquee;
            bar.MarqueeAnimationSpeed = 30;
            bar.SetBounds(20, 52, 340, 18);
            form.Controls.Add(label);
            form.Controls.Add(bar);

            bool ok = false;
            string failure = null;
            form.Shown += delegate
            {
                Thread worker = new Thread(delegate ()
                {
                    try { ok = Install(self, out failure); }
                    catch (Exception e) { Log("error: " + e); failure = e.Message; }
                    form.BeginInvoke(new MethodInvoker(form.Close));
                });
                worker.IsBackground = true;
                worker.Start();
            };
            Application.Run(form);
            if (!ok) ShowError(failure);
            return ok;
        }

        static bool Install(string self, out string failure)
        {
            failure = null;
            string stage = Path.Combine(Path.GetTempPath(), "ArchieOS-Setup-" + Process.GetCurrentProcess().Id + "-" + DateTime.UtcNow.Ticks);
            File.WriteAllText(LogFile, string.Format(CultureInfo.InvariantCulture, "{0:u} ArchieOS setup {1}\r\n{2}\r\n",
                DateTime.UtcNow, Version(), self), Encoding.UTF8);
            try
            {
                Extract(stage);
                string arguments = "-NoProfile -ExecutionPolicy Bypass -File " + Quote(Path.Combine(stage, InstallScript)) + " -SetupExe " + Quote(self);
                ProcessStartInfo info = new ProcessStartInfo(PowerShell(), arguments);
                info.UseShellExecute = false;
                info.CreateNoWindow = true;
                info.RedirectStandardOutput = true;
                info.RedirectStandardError = true;
                // Windows PowerShell writes redirected text in the console's code page: the
                // system's OEM one.
                try
                {
                    Encoding oem = Encoding.GetEncoding((int)GetOEMCP());
                    info.StandardOutputEncoding = oem;
                    info.StandardErrorEncoding = oem;
                }
                catch (Exception) { }
                info.WorkingDirectory = stage;
                StringBuilder output = new StringBuilder();
                int code;
                using (Process ps = new Process())
                {
                    ps.StartInfo = info;
                    DataReceivedEventHandler collect = delegate (object sender, DataReceivedEventArgs e)
                    {
                        if (e.Data == null) return;
                        lock (output) output.AppendLine(e.Data);
                    };
                    ps.OutputDataReceived += collect;
                    ps.ErrorDataReceived += collect;
                    ps.Start();
                    ps.BeginOutputReadLine();
                    ps.BeginErrorReadLine();
                    ps.WaitForExit();
                    code = ps.ExitCode;
                }
                string text;
                lock (output) text = output.ToString();
                Log(text + "exit " + code);
                if (code == 0) return true;
                failure = LastLines(text, 6);
                return false;
            }
            finally
            {
                try { Directory.Delete(stage, true); } catch (Exception) { }
            }
        }

        // The launcher runs in Windows PowerShell 5.1 with WPF, its console hidden, the way
        // ArchieOS.vbs opens it; console tools it starts (wsl.exe) share that hidden console.
        static void OpenLauncher(string target, string[] args)
        {
            StringBuilder arguments = new StringBuilder("-NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -File ");
            arguments.Append(Quote(Path.Combine(target, "ArchieOS-Launcher.ps1")));
            foreach (string a in args) arguments.Append(' ').Append(Quote(a));
            ProcessStartInfo info = new ProcessStartInfo(PowerShell(), arguments.ToString());
            info.UseShellExecute = true;
            info.WindowStyle = ProcessWindowStyle.Hidden;
            info.WorkingDirectory = target;
            Process launcher = Process.Start(info);
            if (launcher != null) launcher.Dispose();
        }

        [DllImport("kernel32.dll")]
        static extern uint GetOEMCP();

        // 64-bit PowerShell on 64-bit Windows: wsl.exe lives only in the real System32.
        static string PowerShell()
        {
            string windows = Environment.GetEnvironmentVariable("WINDIR");
            if (string.IsNullOrEmpty(windows)) windows = @"C:\Windows";
            string system = Environment.Is64BitOperatingSystem && !Environment.Is64BitProcess ? "Sysnative" : "System32";
            return Path.Combine(Path.Combine(windows, system), @"WindowsPowerShell\v1.0\powershell.exe");
        }

        // One argument as CommandLineToArgvW reads it back.
        static string Quote(string arg)
        {
            if (arg.Length > 0 && arg.IndexOfAny(new char[] { ' ', '\t', '"' }) < 0) return arg;
            StringBuilder s = new StringBuilder("\"");
            int slashes = 0;
            foreach (char c in arg)
            {
                if (c == '\\') { slashes++; continue; }
                if (c == '"') s.Append('\\', slashes * 2 + 1).Append('"');
                else s.Append('\\', slashes).Append(c);
                slashes = 0;
            }
            s.Append('\\', slashes * 2).Append('"');
            return s.ToString();
        }

        static string Version()
        {
            object[] info = Assembly.GetExecutingAssembly().GetCustomAttributes(typeof(AssemblyInformationalVersionAttribute), false);
            return info.Length > 0 ? ((AssemblyInformationalVersionAttribute)info[0]).InformationalVersion : "?";
        }

        static string LastLines(string text, int count)
        {
            string[] lines = text.Replace("\r", "").Split(new char[] { '\n' }, StringSplitOptions.RemoveEmptyEntries);
            int from = Math.Max(0, lines.Length - count);
            return string.Join(Environment.NewLine, lines, from, lines.Length - from);
        }

        static void Log(string text)
        {
            try { File.AppendAllText(LogFile, text + "\r\n", Encoding.UTF8); } catch (Exception) { }
        }

        static void ShowError(string detail)
        {
            string message = Text("failed");
            if (!string.IsNullOrEmpty(detail)) message += Environment.NewLine + Environment.NewLine + detail;
            message += Environment.NewLine + Environment.NewLine + LogFile;
            MessageBox.Show(message, "ArchieOS", MessageBoxButtons.OK, MessageBoxIcon.Error);
        }

        // Its two sentences in Windows' display language; the launcher has its own texts.
        static string Text(string key)
        {
            string lang = CultureInfo.CurrentUICulture.TwoLetterISOLanguageName;
            bool installing = key == "installing";
            switch (lang)
            {
                case "es": return installing ? "Instalando el lanzador de ArchieOS…" : "No se pudo instalar el lanzador de ArchieOS.";
                case "pt": return installing ? "Instalando o iniciador do ArchieOS…" : "Não foi possível instalar o iniciador do ArchieOS.";
                case "fr": return installing ? "Installation du lanceur ArchieOS…" : "Le lanceur ArchieOS n'a pas pu être installé.";
                case "de": return installing ? "Der ArchieOS-Launcher wird installiert…" : "Der ArchieOS-Launcher konnte nicht installiert werden.";
                default: return installing ? "Installing the ArchieOS launcher…" : "The ArchieOS launcher could not be installed.";
            }
        }
    }
}

// ArchieOS-Setup.exe: the one file a person downloads. It carries the launcher (its scripts, the
// texts, the model catalog, the signed release manifest) as resources. The first time, and
// whenever it carries a newer launcher than the one installed, it installs it for this Windows
// user with Install-ArchieOS-Launcher.ps1 (no administrator rights; Settings → Apps uninstalls
// it) and opens it on Install. Installed as ArchieOS.exe, it is what the shortcuts open: it
// starts the launcher, passing its arguments through (-Autostart from the Startup folder).
// ARCHIE_SETUP_NO_OPEN=1 installs without opening the launcher (for tests).
//
// Built by scripts/release/Build-Setup.ps1 with the C# compiler that ships with Windows.
using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Windows.Forms;

[assembly: AssemblyTitle("ArchieOS")]
[assembly: AssemblyDescription("ArchieOS: installs and opens the ArchieOS launcher")]
[assembly: AssemblyCompany("ArchieOS")]
[assembly: AssemblyProduct("ArchieOS")]
[assembly: AssemblyCopyright("Apache-2.0 · PentaVM, Xavi and Sheriff")]
[assembly: AssemblyVersion("0.1.0.0")]
[assembly: AssemblyFileVersion("0.1.0.0")]

static class ArchieOSSetup
{
    const string Version = "0.1.0";
    const string Payload = "payload/";

    [STAThread]
    static int Main(string[] args)
    {
        // The same folders Install-ArchieOS-Launcher.ps1 uses: %LOCALAPPDATA%\ArchieOS\Launcher.
        string target = Path.Combine(Folder("LOCALAPPDATA", Environment.SpecialFolder.LocalApplicationData), @"ArchieOS\Launcher");
        // What is installed is this build's launcher when setup.id names this build: every build
        // has its own module id, so a newer setup of the same version still updates the launcher.
        string build = typeof(ArchieOSSetup).Module.ModuleVersionId.ToString();
        string stamp = Path.Combine(target, "setup.id");
        bool installed = File.Exists(Path.Combine(target, "ArchieOS-Launcher.ps1")) && File.Exists(Path.Combine(target, "ArchieOS.vbs"))
            && File.Exists(stamp) && File.ReadAllText(stamp).Trim() == build;
        try
        {
            if (!installed)
            {
                Install(target);
                File.WriteAllText(stamp, build);
                // A first installation opens on Install: checks, WSL, the AI model, the download.
                args = new[] { "-Page", "instalar" };
            }
            if (Environment.GetEnvironmentVariable("ARCHIE_SETUP_NO_OPEN") != "1") Open(target, args);
            return 0;
        }
        catch (Exception error)
        {
            // What went wrong also stays in %TEMP%\ArchieOS-Setup.log, to read or send.
            try { File.WriteAllText(Path.Combine(Path.GetTempPath(), "ArchieOS-Setup.log"), DateTime.Now.ToString("s") + Environment.NewLine + error); } catch { }
            if (Environment.GetEnvironmentVariable("ARCHIE_SETUP_NO_OPEN") != "1")
                MessageBox.Show(error.Message, "ArchieOS", MessageBoxButtons.OK, MessageBoxIcon.Error);
            return 1;
        }
    }

    static string Folder(string variable, Environment.SpecialFolder fallback)
    {
        string value = Environment.GetEnvironmentVariable(variable);
        return string.IsNullOrEmpty(value) ? Environment.GetFolderPath(fallback) : value;
    }

    // The resources go to a folder of their own, Install-ArchieOS-Launcher.ps1 copies them to the
    // launcher's place with its shortcuts, and the folder goes.
    static void Install(string target)
    {
        string stage = Path.Combine(Path.GetTempPath(), "ArchieOS-Setup-" + Version + "-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(stage);
        try
        {
            Assembly self = Assembly.GetExecutingAssembly();
            foreach (string name in self.GetManifestResourceNames())
            {
                if (!name.StartsWith(Payload, StringComparison.Ordinal)) continue;
                string file = name.Substring(Payload.Length);
                if (file.Length == 0 || file.IndexOfAny(new[] { '/', '\\', ':' }) >= 0) continue;
                using (Stream source = self.GetManifestResourceStream(name))
                using (FileStream output = File.Create(Path.Combine(stage, file)))
                    source.CopyTo(output);
            }
            File.Copy(self.Location, Path.Combine(stage, "ArchieOS.exe"), true);

            var start = new ProcessStartInfo("powershell.exe",
                "-NoProfile -ExecutionPolicy Bypass -File \"" + Path.Combine(stage, "Install-ArchieOS-Launcher.ps1") + "\"");
            start.UseShellExecute = false;
            start.CreateNoWindow = true;
            start.RedirectStandardError = true;
            start.RedirectStandardOutput = true;
            using (Process installer = Process.Start(start))
            {
                string output = installer.StandardOutput.ReadToEnd();
                string errors = installer.StandardError.ReadToEnd();
                installer.WaitForExit();
                if (installer.ExitCode != 0 || !File.Exists(Path.Combine(target, "ArchieOS-Launcher.ps1")))
                    throw new Exception("The ArchieOS launcher could not be installed.\n\n" + (errors + output).Trim());
            }
        }
        finally
        {
            try { Directory.Delete(stage, true); } catch { }
        }
    }

    // The launcher is a PowerShell window started without a console, as ArchieOS.vbs does.
    static void Open(string target, string[] args)
    {
        string arguments = "\"" + Path.Combine(target, "ArchieOS.vbs") + "\"";
        foreach (string a in args)
            arguments += " \"" + a.Replace("\"", "") + "\"";
        var start = new ProcessStartInfo(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "wscript.exe"), arguments);
        start.UseShellExecute = false;
        start.WorkingDirectory = target;
        Process.Start(start);
    }
}

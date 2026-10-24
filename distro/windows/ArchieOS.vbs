' Opens the ArchieOS launcher without a console window; its arguments pass through
' (-Autostart from the Startup folder, -Page modelo from the installer).
Set shell = CreateObject("WScript.Shell")
here = CreateObject("Scripting.FileSystemObject").GetParentFolderName(WScript.ScriptFullName)
args = ""
For Each a In WScript.Arguments
  args = args & " " & a
Next
shell.Run "powershell.exe -NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -File """ & here & "\ArchieOS-Launcher.ps1""" & args, 0, False

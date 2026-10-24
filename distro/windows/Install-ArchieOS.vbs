' Opens the ArchieOS installer without a console window. Double-click this file (or
' Install-ArchieOS.cmd); Windows also opens it once after the restart WSL's installation asks for.
Set shell = CreateObject("WScript.Shell")
here = CreateObject("Scripting.FileSystemObject").GetParentFolderName(WScript.ScriptFullName)
args = ""
For Each a In WScript.Arguments
  args = args & " " & a
Next
shell.Run "powershell.exe -NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -File """ & here & "\Install-ArchieOS.ps1""" & args, 0, False

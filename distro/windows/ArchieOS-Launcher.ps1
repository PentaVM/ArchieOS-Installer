# ArchieOS on Windows: starts, restarts and stops the node in WSL (or its VirtualBox VM),
# shows how it is set up, opens the GUI and the voice, and changes the local model.
#
#   ArchieOS-Launcher.ps1              the window
#   ArchieOS-Launcher.ps1 -Autostart   at Windows sign-in: starts the node, then opens the
#                                      GUI if so set, with the window minimized
#   ArchieOS-Launcher.ps1 -Page modelo opens on the model wizard (the installer does this)
#
# Settings: %APPDATA%\ArchieOS\launcher.json. Inside WSL it calls archie-node and
# archie-local, the image's own commands; it holds no logic of its own about the node.
# Windows PowerShell 5.1 and WPF, both part of Windows 10 and 11: nothing to install.
param(
    [switch]$Autostart,
    [string]$Page = ''
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationFramework, PresentationCore, WindowsBase, System.Windows.Forms, System.Drawing

# One launcher at a time: a second one (say, the sign-in one and a click) just leaves.
$mutex = New-Object System.Threading.Mutex($false, 'Local\ArchieOS-Launcher')
if (-not $mutex.WaitOne(0)) { exit 0 }

$settingsDir = Join-Path $env:APPDATA 'ArchieOS'
$settingsFile = Join-Path $settingsDir 'launcher.json'
$startupLink = Join-Path ([Environment]::GetFolderPath('Startup')) 'ArchieOS.lnk'
$vbox = Join-Path $env:ProgramFiles 'Oracle\VirtualBox\VBoxManage.exe'

function Read-Settings {
    $defaults = [ordered]@{
        backend = 'wsl'; distro = ''; vm = 'ArchieOS'
        open_gui_on_start = $true; gui_mode = 'shell'; gui_software = $false
        voice_on_start = $false; voice_always = $true; advanced = $false
        # At sign-in the window says it is about to start, and waits this long for a "no".
        autostart_delay_s = 10; show_all_models = $false
    }
    if (Test-Path $settingsFile) {
        try {
            $saved = Get-Content $settingsFile -Raw -Encoding UTF8 | ConvertFrom-Json
            foreach ($p in $saved.PSObject.Properties) { $defaults[$p.Name] = $p.Value }
        } catch { }
    }
    if (-not $defaults.distro) { $defaults.distro = (Get-WslDistros | Where-Object { $_ -like 'ArchieOS*' } | Select-Object -First 1) }
    return $defaults
}
function Save-Settings {
    New-Item -ItemType Directory -Force $settingsDir | Out-Null
    ($script:cfg | ConvertTo-Json) | Set-Content $settingsFile -Encoding UTF8
}
function Get-WslDistros {
    $raw = (& wsl.exe --list --quiet 2>$null) -join "`n"
    return @(($raw -replace "`0", '') -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ })
}
function Test-DistroRunning([string]$name) {
    $raw = (& wsl.exe --list --running --quiet 2>$null) -join "`n"
    return @(($raw -replace "`0", '') -split "`r?`n" | ForEach-Object { $_.Trim() }) -contains $name
}
# The distribution's virtual disk on Windows and what it takes there: the file only grows,
# so a disk that once held a large build keeps that room until it is compacted.
if (-not ([System.Management.Automation.PSTypeName]'ArchieOS.Native').Type) {
    Add-Type -Namespace ArchieOS -Name Native -MemberDefinition '[DllImport("kernel32.dll", CharSet = CharSet.Unicode)] public static extern uint GetCompressedFileSizeW(string path, out uint high);'
}
function Get-WslDisk([string]$name) {
    try {
        $entry = Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss' -ErrorAction Stop |
            ForEach-Object { Get-ItemProperty $_.PSPath } | Where-Object { $_.DistributionName -eq $name } | Select-Object -First 1
        if (-not $entry) { return $null }
        $vhdx = Join-Path ($entry.BasePath -replace '^\\\\\?\\', '') 'ext4.vhdx'
        if (-not (Test-Path $vhdx)) { return $null }
        $high = [uint32]0
        $low = [ArchieOS.Native]::GetCompressedFileSizeW($vhdx, [ref]$high)
        $drive = (Get-Item $vhdx).PSDrive
        return @{ path = $vhdx; bytes = ([uint64]$high -shl 32) + $low; drive = $drive.Name; free = (Get-PSDrive $drive.Name).Free }
    } catch { return $null }
}
function Get-VBoxVms {
    if (-not (Test-Path $vbox)) { return @() }
    return @(& $vbox list vms 2>$null | ForEach-Object { if ($_ -match '^"(.+)"') { $Matches[1] } })
}

$script:cfg = Read-Settings
# For a VirtualBox machine the model runs on Windows; those functions live beside this file.
foreach ($module in 'ArchieOS-Machine.ps1', 'ArchieOS-HostModel.ps1') {
    $path = Join-Path $PSScriptRoot $module
    if (Test-Path $path) { . $path }
}
if (-not (Get-Command Start-HostModel -ErrorAction SilentlyContinue)) {
    function Start-HostModel($hostModel) { throw 'falta ArchieOS-HostModel.ps1 junto al lanzador' }
    function Test-HostModel($hostModel) { return $false }
    function Stop-HostModel { }
}

# ---- the window -------------------------------------------------------------------------
[xml]$xaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        Title="ArchieOS" Width="980" Height="700" MinWidth="760" MinHeight="560"
        Background="#0D1117" Foreground="#E6EDF3" FontFamily="Segoe UI" FontSize="13"
        WindowStartupLocation="CenterScreen">
  <Window.Resources>
    <Style TargetType="Button">
      <Setter Property="Background" Value="#21262D"/>
      <Setter Property="Foreground" Value="#E6EDF3"/>
      <Setter Property="BorderBrush" Value="#30363D"/>
      <Setter Property="Padding" Value="14,7"/>
      <Setter Property="Margin" Value="0,0,8,0"/>
      <Setter Property="Cursor" Value="Hand"/>
      <Setter Property="Template">
        <Setter.Value>
          <ControlTemplate TargetType="Button">
            <Border x:Name="Box" Background="{TemplateBinding Background}" BorderBrush="{TemplateBinding BorderBrush}"
                    BorderThickness="1" CornerRadius="6" Padding="{TemplateBinding Padding}">
              <ContentPresenter HorizontalAlignment="Center" VerticalAlignment="Center"/>
            </Border>
            <ControlTemplate.Triggers>
              <Trigger Property="IsMouseOver" Value="True"><Setter TargetName="Box" Property="Opacity" Value="0.85"/></Trigger>
              <Trigger Property="IsEnabled" Value="False"><Setter TargetName="Box" Property="Opacity" Value="0.4"/></Trigger>
            </ControlTemplate.Triggers>
          </ControlTemplate>
        </Setter.Value>
      </Setter>
    </Style>
    <Style TargetType="CheckBox"><Setter Property="Foreground" Value="#E6EDF3"/><Setter Property="Margin" Value="0,4"/></Style>
    <Style TargetType="RadioButton"><Setter Property="Foreground" Value="#E6EDF3"/><Setter Property="Margin" Value="0,4,16,4"/></Style>
    <Style TargetType="TextBlock"><Setter Property="TextWrapping" Value="Wrap"/></Style>
    <Style x:Key="Card" TargetType="Border">
      <Setter Property="Background" Value="#161B22"/><Setter Property="BorderBrush" Value="#30363D"/>
      <Setter Property="BorderThickness" Value="1"/><Setter Property="CornerRadius" Value="10"/>
      <Setter Property="Padding" Value="16"/><Setter Property="Margin" Value="0,0,0,12"/>
    </Style>
    <Style x:Key="Label" TargetType="TextBlock">
      <Setter Property="Foreground" Value="#8B949E"/><Setter Property="FontSize" Value="11"/>
      <Setter Property="FontWeight" Value="SemiBold"/><Setter Property="Margin" Value="0,0,0,8"/>
    </Style>
  </Window.Resources>
  <Grid Margin="20">
    <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
    <DockPanel Grid.Row="0" Margin="0,0,0,16">
      <StackPanel Orientation="Horizontal" DockPanel.Dock="Left" VerticalAlignment="Center">
        <TextBlock Text="ArchieOS" FontSize="24" FontWeight="SemiBold"/>
        <Border x:Name="StatePill" CornerRadius="11" Padding="10,3" Margin="14,4,0,0" Background="#30363D" VerticalAlignment="Center">
          <TextBlock x:Name="StateText" Text="Leyendo…" FontSize="12" FontWeight="SemiBold"/>
        </Border>
      </StackPanel>
      <CheckBox x:Name="AdvancedToggle" Content="Avanzado" DockPanel.Dock="Right" HorizontalAlignment="Right" VerticalAlignment="Center"/>
    </DockPanel>

    <Grid Grid.Row="1">
      <Grid.ColumnDefinitions><ColumnDefinition Width="360"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
      <ScrollViewer Grid.Column="0" VerticalScrollBarVisibility="Auto" Margin="0,0,16,0">
      <StackPanel>
        <Border x:Name="AutostartBanner" Visibility="Collapsed" Background="#132235" BorderBrush="#388BFD"
                BorderThickness="1" CornerRadius="10" Padding="14" Margin="0,0,0,12">
          <StackPanel>
            <TextBlock x:Name="AutostartText" FontWeight="SemiBold" Margin="0,0,0,10"/>
            <WrapPanel>
              <Button x:Name="AutostartNow" Content="Iniciar ahora" Background="#1F6FEB" BorderBrush="#388BFD" Margin="0,0,8,8"/>
              <Button x:Name="AutostartSkip" Content="No iniciar esta vez" Margin="0,0,8,8"/>
              <Button x:Name="AutostartNever" Content="No iniciar con Windows" Margin="0,0,8,8"/>
            </WrapPanel>
          </StackPanel>
        </Border>
        <Border Style="{StaticResource Card}">
          <StackPanel>
            <TextBlock Style="{StaticResource Label}" Text="NODO"/>
            <TextBlock x:Name="StateDetail" Text="" Foreground="#8B949E" Margin="0,0,0,14"/>
            <Button x:Name="PrimaryButton" Content="Iniciar" FontSize="16" FontWeight="SemiBold" Padding="18,12"
                    Background="#1F6FEB" BorderBrush="#388BFD" Margin="0,0,0,10" HorizontalAlignment="Stretch"/>
            <WrapPanel>
              <Button x:Name="GuiButton" Content="Abrir GUI" Margin="0,0,8,8"/>
              <Button x:Name="TalkButton" Content="Hablar con Archie" Margin="0,0,8,8"/>
              <Button x:Name="StopButton" Content="Detener" Margin="0,0,8,8"/>
            </WrapPanel>
            <CheckBox x:Name="StartupToggle" Content="Iniciar ArchieOS al entrar a Windows" Margin="0,6,0,0"/>
            <TextBlock x:Name="DiskWarning" Visibility="Collapsed" Foreground="#D29922" FontSize="12" Margin="0,10,0,6"/>
            <Button x:Name="DiskButton" Visibility="Collapsed" Content="Liberar espacio en disco" HorizontalAlignment="Left"/>
          </StackPanel>
        </Border>
        <Border Style="{StaticResource Card}">
          <StackPanel>
            <TextBlock Style="{StaticResource Label}" Text="SERVICIOS"/>
            <ItemsControl x:Name="ServiceList"/>
          </StackPanel>
        </Border>
      </StackPanel>
      </ScrollViewer>

      <Grid Grid.Column="1">
        <Border x:Name="PreviewCard" Style="{StaticResource Card}">
          <ScrollViewer VerticalScrollBarVisibility="Auto">
            <StackPanel>
              <TextBlock Style="{StaticResource Label}" Text="CONFIGURACIÓN"/>
              <Grid x:Name="PreviewGrid">
                <Grid.ColumnDefinitions><ColumnDefinition Width="150"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
              </Grid>
              <TextBlock Margin="0,14,0,0" Foreground="#8B949E" FontSize="12"
                Text="Avanzado muestra el asistente del modelo local, el inicio con Windows, el destino (WSL o VirtualBox) y el registro."/>
            </StackPanel>
          </ScrollViewer>
        </Border>

        <TabControl x:Name="AdvancedTabs" Visibility="Collapsed" Background="#161B22" BorderBrush="#30363D" Foreground="#E6EDF3">
          <TabControl.Resources>
            <!-- A tab's content takes its text color from the TabControl, black by default. -->
            <Style TargetType="TabItem">
              <Setter Property="Foreground" Value="#0D1117"/>
              <Setter Property="Padding" Value="12,5"/>
            </Style>
          </TabControl.Resources>
          <TabItem x:Name="ModelTab" Header="Modelo local">
            <DockPanel Margin="12" TextElement.Foreground="#E6EDF3">
              <TextBlock DockPanel.Dock="Top" x:Name="ModelIntro" Foreground="#8B949E" Margin="0,0,0,6"/>
              <CheckBox DockPanel.Dock="Top" x:Name="ShowAllModels" Content="Mostrar también los que no caben en este equipo" Margin="0,0,0,8"/>
              <DockPanel DockPanel.Dock="Bottom" Margin="0,10,0,0">
                <TextBlock Text="Carpeta de modelos:" VerticalAlignment="Center" Margin="0,0,8,0"/>
                <Button x:Name="ModelsDirApply" Content="Cambiar" DockPanel.Dock="Right" Margin="8,0,0,0"/>
                <TextBox x:Name="ModelsDirBox" Background="#0D1117" Foreground="#E6EDF3" BorderBrush="#30363D" Padding="4"/>
              </DockPanel>
              <ScrollViewer VerticalScrollBarVisibility="Auto"><StackPanel x:Name="ModelList"/></ScrollViewer>
            </DockPanel>
          </TabItem>
          <TabItem Header="Inicio">
            <StackPanel Margin="12" TextElement.Foreground="#E6EDF3">
              <CheckBox x:Name="AutostartBox" Content="Iniciar ArchieOS al entrar a Windows"/>
              <CheckBox x:Name="OpenGuiBox" Content="Abrir la GUI cuando el nodo está en marcha"/>
              <CheckBox x:Name="VoiceOnStartBox" Content="Abrir la voz al iniciar"/>
              <CheckBox x:Name="VoiceAlwaysBox" Content="La voz responde a todo lo que oye (sin decir «Archie»)"/>
              <TextBlock Text="GUI" Margin="0,14,0,4" Foreground="#8B949E"/>
              <StackPanel Orientation="Horizontal">
                <RadioButton x:Name="GuiShell" Content="Escritorio (shell)" GroupName="gui"/>
                <RadioButton x:Name="GuiObservatory" Content="Observatorio" GroupName="gui"/>
              </StackPanel>
              <CheckBox x:Name="GuiSoftwareBox" Content="Dibujar con la CPU (si la GPU está ocupada o la GUI se congela)"/>
              <TextBlock Text="Segundos de aviso antes de iniciar al entrar a Windows" Margin="0,14,0,4" Foreground="#8B949E"/>
              <ComboBox x:Name="DelayBox" Width="120" HorizontalAlignment="Left"/>
              <WrapPanel Margin="0,16,0,0">
                <Button x:Name="SetupButton" Content="Asistente de instalación (voz y clave)" Margin="0,0,8,8"/>
                <Button x:Name="LogsButton" Content="Abrir registros" Margin="0,0,8,8"/>
                <Button x:Name="CompactButton" Content="Liberar espacio en disco" Margin="0,0,8,8"/>
                <Button x:Name="UninstallButton" Content="Desinstalar el lanzador" Margin="0,0,8,8"/>
              </WrapPanel>
            </StackPanel>
          </TabItem>
          <TabItem Header="Destino">
            <StackPanel Margin="12" TextElement.Foreground="#E6EDF3">
              <StackPanel Orientation="Horizontal">
                <RadioButton x:Name="BackendWsl" Content="WSL" GroupName="backend"/>
                <RadioButton x:Name="BackendVbox" Content="VirtualBox" GroupName="backend"/>
              </StackPanel>
              <TextBlock Text="Distribución de WSL" Margin="0,12,0,4" Foreground="#8B949E"/>
              <ComboBox x:Name="DistroBox" Width="360" HorizontalAlignment="Left"/>
              <TextBlock Text="Máquina de VirtualBox" Margin="0,12,0,4" Foreground="#8B949E"/>
              <ComboBox x:Name="VmBox" Width="360" HorizontalAlignment="Left"/>
              <TextBlock x:Name="VboxNote" Margin="0,8,0,0" Foreground="#8B949E"/>
            </StackPanel>
          </TabItem>
          <TabItem Header="Registro">
            <TextBox x:Name="LogBox" Margin="8" IsReadOnly="True" FontFamily="Consolas" FontSize="12"
                     Background="#0D1117" Foreground="#C9D1D9" BorderBrush="#30363D"
                     VerticalScrollBarVisibility="Auto" TextWrapping="Wrap"/>
          </TabItem>
        </TabControl>
      </Grid>
    </Grid>

    <TextBlock Grid.Row="2" x:Name="Footer" Foreground="#8B949E" FontSize="11" Margin="0,8,0,0"/>
  </Grid>
</Window>
'@
$window = [Windows.Markup.XamlReader]::Load((New-Object System.Xml.XmlNodeReader $xaml))
$ui = @{}
$xaml.SelectNodes('//*[@*[local-name()="Name"]]') | ForEach-Object {
    $name = $_.Attributes | Where-Object { $_.LocalName -eq 'Name' } | Select-Object -First 1 -ExpandProperty Value
    $ui[$name] = $window.FindName($name)
}

function Add-Log([string]$text) {
    if (-not $text) { return }
    $ui.LogBox.AppendText(("[{0}] {1}`r`n" -f (Get-Date -Format 'HH:mm:ss'), $text.TrimEnd()))
    $ui.LogBox.ScrollToEnd()
}
function Brush([string]$hex) { return (New-Object System.Windows.Media.BrushConverter).ConvertFromString($hex) }

# ---- running commands without freezing the window ----------------------------------------
# Each command runs hidden through cmd.exe with its output in a temporary file; a timer reads
# that file and, when the process ends, calls what comes next with the whole output, the exit
# code and $data (the callback runs later, when the caller's variables are gone).
$script:jobs = New-Object System.Collections.ArrayList
function Start-Command([string]$file, [string]$arguments, [scriptblock]$then, [string]$label = '', $data = $null) {
    $out = [System.IO.Path]::GetTempFileName()
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = 'cmd.exe'
    $psi.Arguments = "/d /c `"`"$file`" $arguments > `"$out`" 2>&1`""
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $p = [System.Diagnostics.Process]::Start($psi)
    [void]$script:jobs.Add(@{ process = $p; out = $out; then = $then; label = $label; read = 0; data = $data })
}
function Read-Output([string]$path) {
    try {
        $bytes = [System.IO.File]::ReadAllBytes($path)
        $text = [System.Text.Encoding]::UTF8.GetString($bytes)
        return ($text -replace "`0", '')
    } catch { return '' }
}
function Wsl([string]$command, [scriptblock]$then, [switch]$root, [string]$label = '', $data = $null) {
    $user = if ($root) { '-u root' } else { '' }
    # Unquoted: through cmd.exe a quoted name reaches wsl.exe with its quotes and is not found.
    # WSL's own names have no spaces; anything else is refused here.
    if ($script:cfg.distro -notmatch '^[A-Za-z0-9._-]+$') { Add-Log "Nombre de distribución no válido: $($script:cfg.distro)"; return }
    Start-Command 'wsl.exe' ("-d {0} {1} -- {2}" -f $script:cfg.distro, $user, $command) $then $label $data
}
$pump = New-Object System.Windows.Threading.DispatcherTimer
$pump.Interval = [TimeSpan]::FromMilliseconds(400)
$pump.Add_Tick({
    foreach ($job in @($script:jobs)) {
        $text = Read-Output $job.out
        if ($job.label -and $text.Length -gt $job.read) {
            ($text.Substring($job.read) -split "`r?`n") | Where-Object { $_.Trim() } | ForEach-Object { Add-Log $_ }
            $job.read = $text.Length
        }
        if ($job.process.HasExited) {
            $script:jobs.Remove($job)
            Remove-Item $job.out -ErrorAction SilentlyContinue
            # An error in what comes next goes to the log; uncaught, it would close the window.
            if ($job.then) {
                try { & $job.then $text $job.process.ExitCode $job.data }
                catch { Add-Log "Error: $($_.Exception.Message) (línea $($_.InvocationInfo.ScriptLineNumber))"; $script:refreshing = $false; $script:busy = '' }
            }
        }
    }
})
$pump.Start()

# ---- state ------------------------------------------------------------------------------
$script:status = $null      # archie-node status --json
$script:config = $null      # archie-node config --json
$script:models = @()        # archie-local list --json
$script:busy = ''           # what is under way: '', 'start', 'restart', 'stop', 'model'
$serviceNames = [ordered]@{ model = 'Modelo local'; archied = 'Core (archied)'; displayd = 'displayd'; worldd = 'worldd'; wanda = 'Wanda'; postgres = 'PostgreSQL'; gui = 'GUI' }

function Get-NodeState {
    if ($script:cfg.backend -eq 'virtualbox') {
        if (-not (Test-Path $vbox)) { return 'unavailable' }
        $info = & $vbox showvminfo $script:cfg.vm --machinereadable 2>$null
        if (-not $info) { return 'unavailable' }
        if ($info -match 'VMState="running"') { return 'running' } else { return 'stopped' }
    }
    if (-not $script:cfg.distro) { return 'unavailable' }
    if (-not $script:status) { return 'stopped' }
    $s = $script:status.services
    $needed = @('model', 'archied', 'displayd', 'worldd')
    if ($script:status.wanda_configured) { $needed += 'wanda' }
    $up = @($needed | Where-Object { $s.$_ })
    if ($up.Count -eq $needed.Count) { return 'running' }
    if ($up.Count -eq 0) { return 'stopped' }
    return 'partial'
}

function Update-View {
    $state = Get-NodeState
    $labels = @{ running = 'En marcha'; stopped = 'Detenido'; partial = 'En marcha a medias'; unavailable = 'Sin destino' }
    $colors = @{ running = '#238636'; stopped = '#30363D'; partial = '#9E6A03'; unavailable = '#6E2B2B' }
    $busyText = @{ start = 'Iniciando…'; restart = 'Reiniciando…'; stop = 'Deteniendo…'; model = 'Cambiando de modelo…'; gui = 'Abriendo la GUI…'; compact = 'Liberando espacio…' }
    if ($script:busy) {
        $ui.StateText.Text = $busyText[$script:busy]; $ui.StatePill.Background = Brush '#1F6FEB'
    } else {
        $ui.StateText.Text = $labels[$state]; $ui.StatePill.Background = Brush $colors[$state]
    }
    $target = if ($script:cfg.backend -eq 'virtualbox') { "VirtualBox · $($script:cfg.vm)" } else { "WSL · $($script:cfg.distro)" }
    $detail = @{
        running = 'Todo responde. Reiniciar lo apaga y lo vuelve a levantar.'
        stopped = 'Nada corre todavía. Iniciar levanta la base de datos, el modelo local, Core, Wanda y la vista.'
        partial = 'Algunos servicios no responden (abajo). Reiniciar los vuelve a levantar.'
        unavailable = 'No encuentro la distribución o la máquina virtual. Elígela en Avanzado → Destino.'
    }
    $ui.StateDetail.Text = "$target`n$($detail[$state])"
    $ui.PrimaryButton.Content = if ($state -eq 'stopped') { 'Iniciar' } else { 'Reiniciar' }
    $idle = -not $script:busy
    $ui.PrimaryButton.IsEnabled = $idle -and $state -ne 'unavailable'
    $ui.StopButton.IsEnabled = $idle -and ($state -eq 'running' -or $state -eq 'partial')
    $ui.GuiButton.IsEnabled = $idle -and ($state -eq 'running' -or $state -eq 'partial')
    $ui.TalkButton.IsEnabled = $idle -and $script:cfg.backend -eq 'wsl' -and ($state -eq 'running' -or $state -eq 'partial')

    $ui.ServiceList.Items.Clear()
    foreach ($key in $serviceNames.Keys) {
        if ($key -eq 'wanda' -and $script:status -and -not $script:status.wanda_configured) { continue }
        $up = $script:status -and $script:status.services.$key
        $row = New-Object System.Windows.Controls.StackPanel
        $row.Orientation = 'Horizontal'; $row.Margin = '0,3'
        $dot = New-Object System.Windows.Shapes.Ellipse
        $dot.Width = 9; $dot.Height = 9; $dot.Margin = '0,0,10,0'
        $dot.Fill = if ($up) { Brush '#3FB950' } else { Brush '#484F58' }
        $name = New-Object System.Windows.Controls.TextBlock
        $name.Text = $serviceNames[$key]
        if ($key -eq 'model' -and $script:status -and $script:status.model) { $name.Text = "Modelo local · $($script:status.model.name)" }
        [void]$row.Children.Add($dot); [void]$row.Children.Add($name)
        [void]$ui.ServiceList.Items.Add($row)
    }
    Update-Preview
    Update-Disk
    $ui.StartupToggle.IsChecked = Test-Path $startupLink
    $ui.Footer.Text = "Ajustes: $settingsFile · Inicio con Windows: $(if (Test-Path $startupLink) { 'sí' } else { 'no' })"
}

# Says when the virtual disk holds much more than the distribution uses, or the drive it is
# on is nearly full: both are what froze WSL on this project's own machine.
function Update-Disk {
    $disk = if ($script:cfg.backend -eq 'wsl' -and $script:cfg.distro) { Get-WslDisk $script:cfg.distro } else { $null }
    $script:disk = $disk
    $text = ''
    if ($disk) {
        $used = if ($script:config -and $script:config.disk_used_bytes) { [double]$script:config.disk_used_bytes } else { 0 }
        $spare = [double]$disk.bytes - $used
        if ($used -gt 0 -and $spare -gt 20e9) {
            $text = ("El disco de WSL ocupa {0} en {1}: y Linux usa {2}. Liberar espacio devuelve unos {3} a Windows (detiene el nodo unos minutos)." -f `
                (Format-Bytes $disk.bytes), $disk.drive, (Format-Bytes $used), (Format-Bytes $spare))
        } elseif ($disk.free -lt 8e9) {
            $text = ("Quedan {0} libres en {1}:. Con el disco lleno WSL se congela; libera espacio en Windows o compacta el disco de WSL." -f (Format-Bytes $disk.free), $disk.drive)
        }
    }
    $ui.DiskWarning.Text = $text
    $ui.DiskWarning.Visibility = if ($text) { 'Visible' } else { 'Collapsed' }
    $ui.DiskButton.Visibility = if ($text) { 'Visible' } else { 'Collapsed' }
    $ui.DiskButton.IsEnabled = -not $script:busy
    $ui.CompactButton.IsEnabled = -not $script:busy -and [bool]$disk
}

function Format-Bytes([double]$b) { if ($b -ge 1e9) { '{0:N1} GB' -f ($b / 1e9) } else { '{0:N0} MB' -f ($b / 1e6) } }

function Update-Preview {
    $grid = $ui.PreviewGrid
    $grid.Children.Clear(); $grid.RowDefinitions.Clear()
    $rows = New-Object System.Collections.ArrayList
    $target = if ($script:cfg.backend -eq 'virtualbox') { "VirtualBox: $($script:cfg.vm)" } else { "WSL: $($script:cfg.distro)" }
    [void]$rows.Add(@('Destino', $target))
    $c = $script:config
    $l = if ($c) { $c.local } else { $null }
    if ($script:status -and $script:status.model) {
        [void]$rows.Add(@('Modelo local', "$($script:status.model.name) · $($script:status.model.endpoint)"))
    } elseif ($l) { [void]$rows.Add(@('Modelo local', "$($l.chosen) (detenido)")) }
    if ($l) {
        [void]$rows.Add(@('GPU', ("{0} · {1:N1} de {2:N1} GB en uso" -f $l.gpu, ($l.vram_used_mb / 1024), ($l.vram_total_mb / 1024))))
        [void]$rows.Add(@('Memoria de WSL', ('{0:N1} GB' -f ($l.ram_total_mb / 1024))))
        [void]$rows.Add(@('Modelos en', "$($l.models_dir) · $(Format-Bytes $l.free_bytes) libres de verdad"))
        [void]$rows.Add(@('Runtime', $l.runtime))
    }
    if ($c) {
        [void]$rows.Add(@('Core', $c.node_root))
        [void]$rows.Add(@('Wanda', $(if ($c.wanda_dir) { $c.wanda_dir } else { 'no configurada' })))
        [void]$rows.Add(@('Voces', ($c.voices -replace ',', ', ')))
        [void]$rows.Add(@('Escucha', 'español e inglés (auto); responde en el idioma que oye'))
    }
    if ($script:status) {
        [void]$rows.Add(@('Pausa de la IA', $(if ($script:status.ai_paused -eq $true) { 'en pausa' } elseif ($script:status.ai_paused -eq $false) { 'no' } else { 'sin lectura' })))
    }
    [void]$rows.Add(@('Al iniciar', ("{0}{1}" -f $(if ($script:cfg.open_gui_on_start) { "abre la GUI ($($script:cfg.gui_mode))" } else { 'no abre la GUI' }), $(if ($script:cfg.voice_on_start) { ' y la voz' } else { '' }))))
    [void]$rows.Add(@('Inicio con Windows', $(if (Test-Path $startupLink) { 'sí' } else { 'no' })))
    if (-not $c) { [void]$rows.Add(@('', 'El resto aparece cuando el nodo responde.')) }
    $i = 0
    foreach ($r in $rows) {
        $grid.RowDefinitions.Add((New-Object System.Windows.Controls.RowDefinition))
        $k = New-Object System.Windows.Controls.TextBlock; $k.Text = $r[0]; $k.Foreground = Brush '#8B949E'; $k.Margin = '0,4'
        $v = New-Object System.Windows.Controls.TextBlock; $v.Text = $r[1]; $v.Margin = '0,4'
        [System.Windows.Controls.Grid]::SetRow($k, $i); [System.Windows.Controls.Grid]::SetRow($v, $i)
        [System.Windows.Controls.Grid]::SetColumn($v, 1)
        [void]$grid.Children.Add($k); [void]$grid.Children.Add($v)
        $i++
    }
}

# ---- the model wizard -------------------------------------------------------------------
# What fits is judged from the model's size against the GPU and WSL's memory, with room for
# its cache and for Windows; measured on the RTX 3070: Bonsai 2 27B (5.9 GB) put 63 of 64
# layers on the 8 GB card. A runtime that does not know the architecture cannot run it.
function Get-Fit($m, $l) {
    if (-not $m.runnable) { return @('Este runtime no lo soporta todavía', '#6E2B2B') }
    $sizeMb = $m.bytes / 1MB
    $vram = [double]$l.vram_total_mb
    $ram = [double]$l.ram_total_mb
    # One memory for CPU and GPU (DGX Spark, RTX Spark): the model, its cache and the system
    # share it, so 8 GB stay for the rest.
    if ($l.unified_memory -eq $true) {
        if ($sizeMb + 8192 -le $vram) { return @('Cabe en la memoria unificada', '#238636') }
        return @('No cabe en este equipo', '#6E2B2B')
    }
    if ($vram -gt 0 -and $sizeMb + 1500 -le $vram) { return @('Cabe en la GPU', '#238636') }
    if ($vram -gt 0 -and $sizeMb + 1500 -le $vram + 0.5 * $ram) { return @('GPU y CPU, más lento', '#9E6A03') }
    if ($vram -le 0 -and $sizeMb -le 0.5 * $ram) { return @('Solo CPU, lento', '#9E6A03') }
    return @('No cabe en este equipo', '#6E2B2B')
}
function Update-Models {
    $ui.ModelList.Children.Clear()
    $l = if ($script:config) { $script:config.local } else { $null }
    if (-not $l -or -not $script:models) {
        $ui.ModelIntro.Text = 'La lista aparece cuando la distribución responde (Iniciar).'
        return
    }
    $memory = if ($l.unified_memory -eq $true) { '{0:N0} GB de memoria unificada' -f ($l.vram_total_mb / 1024) } `
              else { '{0:N1} GB de GPU · {1:N1} GB de memoria en WSL' -f ($l.vram_total_mb / 1024), ($l.ram_total_mb / 1024) }
    $hidden = 0
    if (-not $ui.ModelsDirBox.IsKeyboardFocused) { $ui.ModelsDirBox.Text = $l.models_dir }
    foreach ($m in $script:models) {
        $fit = Get-Fit $m $l
        # What cannot run here is left out unless asked for; what is already here never is.
        if (-not $script:cfg.show_all_models -and -not $m.here -and $fit[1] -eq '#6E2B2B') { $hidden++; continue }
        $card = New-Object System.Windows.Controls.Border
        $card.Background = Brush $(if ($m.answering) { '#0F2A1A' } else { '#0D1117' })
        $card.BorderBrush = Brush $(if ($m.chosen) { '#1F6FEB' } else { '#30363D' })
        $card.BorderThickness = 1; $card.CornerRadius = 8; $card.Padding = '12,8'; $card.Margin = '0,0,0,6'
        $dock = New-Object System.Windows.Controls.DockPanel
        $buttons = New-Object System.Windows.Controls.StackPanel
        $buttons.Orientation = 'Horizontal'; [System.Windows.Controls.DockPanel]::SetDock($buttons, 'Right')
        $info = New-Object System.Windows.Controls.StackPanel
        $title = New-Object System.Windows.Controls.TextBlock
        $title.Text = "$($m.label)  ·  $(Format-Bytes $m.bytes)"; $title.FontWeight = 'SemiBold'
        $sub = New-Object System.Windows.Controls.TextBlock
        $sub.Foreground = Brush '#8B949E'; $sub.FontSize = 12
        $where = if ($m.answering) { "responde en :$($m.port)" } elseif ($m.here) { 'en este equipo' } `
                 elseif ($m.fetched -gt 0) { ('descargando · {0:P0}' -f ($m.fetched / $m.bytes)) } else { 'no descargado' }
        $licence = if ($m.license) { " · licencia $($m.license)" } else { '' }
        $pieces = if ($m.parts -gt 1) { " · $($m.parts) partes" } else { '' }
        $sub.Text = "$($m.repo)$licence$pieces · $where"
        $badge = New-Object System.Windows.Controls.TextBlock
        $badge.Text = $fit[0]; $badge.Foreground = Brush $fit[1]; $badge.FontSize = 12; $badge.FontWeight = 'SemiBold'
        [void]$info.Children.Add($title); [void]$info.Children.Add($sub); [void]$info.Children.Add($badge)
        $name = $m.name
        if ($m.here) {
            $use = New-Object System.Windows.Controls.Button
            $use.Content = $(if ($m.answering) { 'En uso' } else { 'Usar' })
            $use.IsEnabled = (-not $m.answering) -and $m.runnable -and -not $script:busy -and $fit[0] -ne 'No cabe en este equipo'
            $use.Tag = $name
            $use.Add_Click({ param($sender) Invoke-UseModel $sender.Tag })
            [void]$buttons.Children.Add($use)
            if (-not $m.answering) {
                $rm = New-Object System.Windows.Controls.Button
                $rm.Content = 'Borrar'; $rm.IsEnabled = -not $script:busy
                $rm.Tag = $name
                $rm.Add_Click({ param($sender) Invoke-RemoveModel $sender.Tag })
                [void]$buttons.Children.Add($rm)
            }
        } else {
            $dl = New-Object System.Windows.Controls.Button
            # Only what is still to come needs room; a download cut short resumes.
            $room = [double]$l.free_bytes -gt ([double]$m.bytes - [double]$m.fetched + 1e9)
            $fetching = $script:fetching.ContainsKey($name)
            $dl.Content = $(if ($fetching) { 'Descargando…' } elseif (-not $room) { 'Sin espacio' } elseif ($m.fetched -gt 0) { 'Continuar descarga' } else { 'Descargar' })
            $dl.IsEnabled = $room -and $m.runnable -and -not $fetching -and -not $script:busy
            $dl.Tag = $name
            $dl.Add_Click({ param($sender) Invoke-FetchModel $sender.Tag })
            [void]$buttons.Children.Add($dl)
        }
        [void]$dock.Children.Add($buttons); [void]$dock.Children.Add($info)
        $card.Child = $dock
        [void]$ui.ModelList.Children.Add($card)
    }
    $ui.ModelIntro.Text = ("Uno responde a la vez. {0} · {1} · {2} libres en la carpeta de modelos.{3}" -f `
        $l.gpu, $memory, (Format-Bytes $l.free_bytes), $(if ($hidden -gt 0) { " $hidden más no caben en este equipo." } else { '' }))
}
# Core reads its model manifests when it starts: after a model arrives or is chosen, the
# node installs the manifest of any model that has none yet, and restarts Core if it did.
function Sync-Manifests {
    Wsl 'archie-node models --restart' { param($out, $code) if ($out -notmatch '^archie-node: 0 manifest') { Add-Log $out } }
}
function Invoke-UseModel([string]$name) {
    $script:busy = 'model'; Update-View; Update-Models
    Add-Log "archie-local use $name"
    Wsl "archie-local use $name" { param($out, $code, $model)
        $script:busy = ''
        if ($code -ne 0) { [System.Windows.MessageBox]::Show("No se pudo cambiar a $model.`n`n$out", 'ArchieOS') | Out-Null }
        else { Sync-Manifests }
        Refresh-All
    } -label 'model' -data $name
}
$script:fetching = @{}
function Invoke-FetchModel([string]$name) {
    Add-Log "archie-local setup $name"
    $script:fetching[$name] = $true
    Wsl "archie-local setup $name" { param($out, $code, $model)
        $script:fetching.Remove($model)
        if ($code -ne 0) { [System.Windows.MessageBox]::Show("La descarga de $model no terminó. Se puede continuar donde quedó.`n`n$out", 'ArchieOS') | Out-Null }
        else { Sync-Manifests; $tray.ShowBalloonTip(4000, 'ArchieOS', "$model está descargado. Usar lo pone a responder.", 'Info') }
        Refresh-All
    } -label 'fetch' -data $name
    Refresh-All
}
function Invoke-RemoveModel([string]$name) {
    $answer = [System.Windows.MessageBox]::Show("¿Borrar $name de este equipo? Se puede volver a descargar.", 'ArchieOS', 'YesNo', 'Question')
    if ($answer -ne 'Yes') { return }
    Wsl "archie-local remove $name" { param($out, $code) Add-Log $out; Refresh-All }
}

# ---- reading the node -------------------------------------------------------------------
$script:refreshing = $false
function Refresh-All {
    if ($script:refreshing) { return }
    if ($script:cfg.backend -eq 'virtualbox') {
        # The machine tells VirtualBox what runs in it (archie-guest-status); the model is
        # the server on Windows itself.
        $script:config = $null
        if (-not (Test-Path $vbox)) { $script:status = $null; Update-View; return }
        $script:refreshing = $true
        Start-Command $vbox "guestproperty get `"$($script:cfg.vm)`" /ArchieOS/Status" { param($out, $code)
            $services = @{}
            if ($out -match 'Value:\s*(\S+)') {
                foreach ($pair in $Matches[1] -split ',') { $k, $v = $pair -split '=', 2; $services[$k] = $v -eq 'active' }
            }
            $hostModel = $script:cfg.host_model
            $services['model'] = [bool]($hostModel -and (Test-HostModel $hostModel))
            $script:status = if ($services.Count -gt 1) {
                [pscustomobject]@{ services = [pscustomobject]$services; wanda_configured = $false
                    model = $(if ($hostModel) { [pscustomobject]@{ name = $hostModel.name; endpoint = "Windows :$($hostModel.port)" } }) }
            } else { $null }
            $script:refreshing = $false
            Update-View
        }
        return
    }
    if (-not $script:cfg.distro -or -not (Test-DistroRunning $script:cfg.distro)) {
        # A stopped distribution is not started just to be read.
        $script:status = $null; Update-View; Update-Models; return
    }
    $script:refreshing = $true
    Wsl 'archie-node status --json' { param($out, $code)
        try { $script:status = ($out.Trim() -split "`n" | Select-Object -Last 1) | ConvertFrom-Json } catch { $script:status = $null }
        Wsl 'archie-node config --json' { param($out, $code)
            try { $script:config = ($out.Trim() -split "`n" | Select-Object -Last 1) | ConvertFrom-Json } catch { }
            Wsl 'archie-local list --json' { param($out, $code)
                # Windows PowerShell's ConvertFrom-Json hands a JSON array over as one object:
                # held in a variable and piped, it comes apart into its models.
                try { $parsed = ($out.Trim() -split "`n" | Select-Object -Last 1) | ConvertFrom-Json; $script:models = @($parsed | ForEach-Object { $_ }) } catch { }
                $script:refreshing = $false
                Update-View; Update-Models
            }
        }
    }
}

# ---- actions ----------------------------------------------------------------------------
function Open-Gui {
    $soft = if ($script:cfg.gui_software) { ' --software' } else { '' }
    $script:busy = 'gui'; Update-View
    Wsl "archie-node gui $($script:cfg.gui_mode)$soft" { param($out, $code) $script:busy = ''; Refresh-All } -label 'gui'
}
function Open-Voice {
    $always = if ($script:cfg.voice_always) { ' --always' } else { '' }
    Start-Process wsl.exe -ArgumentList "-d $($script:cfg.distro) -- archie-talk --local$always"
}
function After-Start {
    $script:busy = ''
    Refresh-All
    if ($script:cfg.open_gui_on_start) { Open-Gui }
    if ($script:cfg.voice_on_start) { Open-Voice }
}
function Start-Node([string]$verb) {
    $script:busy = if ($verb -eq 'restart') { 'restart' } else { 'start' }
    Update-View
    if ($script:cfg.backend -eq 'virtualbox') {
        # The model first, on Windows' GPU: the machine asks it as soon as it is up.
        if ($script:cfg.host_model) {
            try { Start-HostModel $script:cfg.host_model; Add-Log "Modelo local en Windows: $($script:cfg.host_model.name) en :$($script:cfg.host_model.port)" }
            catch { Add-Log "El modelo local no arrancó: $($_.Exception.Message)" }
        }
        # The machine's own window is ArchieOS, full screen; without the GUI, it runs unseen.
        $type = if ($script:cfg.open_gui_on_start) { 'gui' } else { 'headless' }
        if ($verb -eq 'restart') { Start-Command $vbox "controlvm `"$($script:cfg.vm)`" reset" { param($o, $c) Add-Log $o; $script:busy = ''; Update-View } 'vbox' }
        else { Start-Command $vbox "startvm `"$($script:cfg.vm)`" --type $type" { param($o, $c) Add-Log $o; $script:busy = ''; Update-View } 'vbox' }
        return
    }
    Add-Log "archie-node $verb en $($script:cfg.distro)"
    $next = if ($verb -eq 'restart') { 'restart' } else { 'up' }
    Wsl 'archie-node db' { param($out, $code, $then)
        Wsl "archie-node $then" { param($out, $code)
            if ($code -ne 0) { Add-Log 'Algún servicio no levantó: ver arriba.' }
            After-Start
        } -label 'node'
    } -root -label 'db' -data $next
}
# Compacting the virtual disk needs WSL stopped and an administrator: it runs elevated, in
# its own window, and the node comes back up after it.
function Invoke-Compact {
    $script_ = Join-Path $PSScriptRoot 'Compact-ArchieOS-Disk.ps1'
    if (-not (Test-Path $script_)) { [System.Windows.MessageBox]::Show("No encuentro $script_.", 'ArchieOS') | Out-Null; return }
    $answer = [System.Windows.MessageBox]::Show("Liberar espacio detiene el nodo y WSL unos minutos, compacta su disco y lo vuelve a iniciar. Windows pedirá permiso de administrador.`n`n¿Seguir?", 'ArchieOS', 'YesNo', 'Question')
    if ($answer -ne 'Yes') { return }
    $script:busy = 'compact'; Update-View
    Add-Log 'Liberando espacio: compactando el disco de WSL'
    try {
        $script:compact = Start-Process powershell.exe -Verb RunAs -PassThru -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$script_`"", '-Distro', $script:cfg.distro
    } catch {
        Add-Log 'No se dio el permiso de administrador: no se compactó nada.'
        $script:busy = ''; Update-View; return
    }
    $wait = New-Object System.Windows.Threading.DispatcherTimer
    $wait.Interval = [TimeSpan]::FromSeconds(2)
    $wait.Add_Tick({
        if (-not $script:compact.HasExited) { return }
        $this.Stop()
        $script:busy = ''
        $drive = if ($script:disk) { $script:disk.drive } else { 'C' }
        Add-Log ('Compactación terminada. {0}: tiene {1} libres.' -f $drive, (Format-Bytes (Get-PSDrive $drive).Free))
        Start-Node 'up'
    })
    $wait.Start()
}
function Stop-Node {
    $script:busy = 'stop'; Update-View
    if ($script:cfg.backend -eq 'virtualbox') {
        Start-Command $vbox "controlvm `"$($script:cfg.vm)`" acpipowerbutton" { param($o, $c) Add-Log $o; Stop-HostModel; $script:busy = ''; Update-View } 'vbox'
        return
    }
    Wsl 'archie-node down' { param($out, $code) $script:busy = ''; Refresh-All } -label 'node'
}

$ui.PrimaryButton.Add_Click({ if ((Get-NodeState) -eq 'stopped') { Start-Node 'up' } else { Start-Node 'restart' } })
$ui.StopButton.Add_Click({ Stop-Node })
$ui.GuiButton.Add_Click({
    if ($script:cfg.backend -eq 'virtualbox') { Start-Process $vbox -ArgumentList "startvm `"$($script:cfg.vm)`" --type separate" } else { Open-Gui }
})
$ui.TalkButton.Add_Click({ Open-Voice })
$ui.SetupButton.Add_Click({ Start-Process wsl.exe -ArgumentList "-d $($script:cfg.distro) -- bash -lc `"archie-setup; read -p 'Enter para cerrar'`"" })
$ui.LogsButton.Add_Click({ Start-Process explorer.exe ("\\wsl$\{0}\var\tmp\archie-logs" -f $script:cfg.distro) })
$ui.ModelsDirApply.Add_Click({
    $dir = $ui.ModelsDirBox.Text.Trim()
    if (-not $dir.StartsWith('/')) { [System.Windows.MessageBox]::Show('La carpeta es una ruta de Linux, como /mnt/d/ArchieLocal/models.', 'ArchieOS') | Out-Null; return }
    Wsl "archie-local config models-dir '$dir'" { param($out, $code) Add-Log $out; Refresh-All } -root
})

# Settings controls.
$ui.AdvancedToggle.IsChecked = [bool]$script:cfg.advanced
$ui.AutostartBox.IsChecked = Test-Path $startupLink
$ui.OpenGuiBox.IsChecked = [bool]$script:cfg.open_gui_on_start
$ui.VoiceOnStartBox.IsChecked = [bool]$script:cfg.voice_on_start
$ui.VoiceAlwaysBox.IsChecked = [bool]$script:cfg.voice_always
$ui.GuiShell.IsChecked = $script:cfg.gui_mode -ne 'observatorio'
$ui.GuiObservatory.IsChecked = $script:cfg.gui_mode -eq 'observatorio'
$ui.GuiSoftwareBox.IsChecked = [bool]$script:cfg.gui_software
$ui.BackendWsl.IsChecked = $script:cfg.backend -ne 'virtualbox'
$ui.BackendVbox.IsChecked = $script:cfg.backend -eq 'virtualbox'
foreach ($d in Get-WslDistros) { [void]$ui.DistroBox.Items.Add($d) }
$ui.DistroBox.SelectedItem = $script:cfg.distro
$vms = Get-VBoxVms
foreach ($v in $vms) { [void]$ui.VmBox.Items.Add($v) }
if ($vms -contains $script:cfg.vm) { $ui.VmBox.SelectedItem = $script:cfg.vm }
$ui.VboxNote.Text = if (-not (Test-Path $vbox)) { 'VirtualBox no está instalado.' } `
    elseif ($vms.Count -eq 0) { 'VirtualBox está instalado pero no tiene máquinas. La imagen OVA de ArchieOS aún no se publica; cuando exista, aquí se elige y se enciende, apaga o reinicia.' } `
    else { 'En VirtualBox el lanzador enciende, apaga y reinicia la máquina; Abrir GUI muestra su pantalla. Los servicios los levanta la propia imagen al arrancar.' }

function Set-Autostart([bool]$on) {
    if ($on) {
        $shell = New-Object -ComObject WScript.Shell
        $lnk = $shell.CreateShortcut($startupLink)
        $lnk.TargetPath = Join-Path $env:WINDIR 'System32\wscript.exe'
        $lnk.Arguments = "`"$(Join-Path $PSScriptRoot 'ArchieOS.vbs')`" -Autostart"
        $lnk.WorkingDirectory = $PSScriptRoot
        $lnk.Description = 'ArchieOS: levanta el nodo al entrar a Windows'
        $lnk.Save()
    } else { Remove-Item $startupLink -ErrorAction SilentlyContinue }
}
# Checked and Unchecked, not Click: they also come from the keyboard and from accessibility tools.
$onAdvanced = {
    $script:cfg.advanced = [bool]$ui.AdvancedToggle.IsChecked; Save-Settings
    $ui.AdvancedTabs.Visibility = if ($script:cfg.advanced) { 'Visible' } else { 'Collapsed' }
    $ui.PreviewCard.Visibility = if ($script:cfg.advanced) { 'Collapsed' } else { 'Visible' }
    if ($script:cfg.advanced) { Update-Models }
}
$ui.AdvancedToggle.Add_Checked($onAdvanced)
$ui.AdvancedToggle.Add_Unchecked($onAdvanced)
# The same setting in two places, the main card and Advanced → Inicio; each follows the other.
$ui.AutostartBox.Add_Click({ Set-Autostart ([bool]$ui.AutostartBox.IsChecked); Update-View })
$ui.StartupToggle.Add_Click({ Set-Autostart ([bool]$ui.StartupToggle.IsChecked); $ui.AutostartBox.IsChecked = $ui.StartupToggle.IsChecked; Update-View })
foreach ($s in 0, 5, 10, 20, 30, 60) { [void]$ui.DelayBox.Items.Add("$s s") }
$ui.DelayBox.SelectedItem = "$([int]$script:cfg.autostart_delay_s) s"
$ui.DelayBox.Add_SelectionChanged({ if ($ui.DelayBox.SelectedItem) { $script:cfg.autostart_delay_s = [int](([string]$ui.DelayBox.SelectedItem) -replace '\D', ''); Save-Settings } })
$ui.ShowAllModels.IsChecked = [bool]$script:cfg.show_all_models
$ui.ShowAllModels.Add_Click({ $script:cfg.show_all_models = [bool]$ui.ShowAllModels.IsChecked; Save-Settings; Update-Models })
$ui.CompactButton.Add_Click({ Invoke-Compact })
$ui.DiskButton.Add_Click({ Invoke-Compact })
$ui.UninstallButton.Add_Click({
    $uninstall = Join-Path $PSScriptRoot 'Uninstall-ArchieOS-Launcher.ps1'
    if (-not (Test-Path $uninstall)) { [System.Windows.MessageBox]::Show("No encuentro $uninstall.", 'ArchieOS') | Out-Null; return }
    $answer = [System.Windows.MessageBox]::Show("¿Desinstalar el lanzador? Se quitan sus accesos directos, el inicio con Windows y sus ajustes. La distribución de WSL, el nodo y los modelos se quedan.", 'ArchieOS', 'YesNo', 'Question')
    if ($answer -ne 'Yes') { return }
    Start-Process powershell.exe -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$uninstall`"", '-Quiet'
    $script:quit = $true; $window.Close()
})
$ui.OpenGuiBox.Add_Click({ $script:cfg.open_gui_on_start = [bool]$ui.OpenGuiBox.IsChecked; Save-Settings; Update-View })
$ui.VoiceOnStartBox.Add_Click({ $script:cfg.voice_on_start = [bool]$ui.VoiceOnStartBox.IsChecked; Save-Settings; Update-View })
$ui.VoiceAlwaysBox.Add_Click({ $script:cfg.voice_always = [bool]$ui.VoiceAlwaysBox.IsChecked; Save-Settings })
$ui.GuiShell.Add_Click({ $script:cfg.gui_mode = 'shell'; Save-Settings; Update-View })
$ui.GuiObservatory.Add_Click({ $script:cfg.gui_mode = 'observatorio'; Save-Settings; Update-View })
$ui.GuiSoftwareBox.Add_Click({ $script:cfg.gui_software = [bool]$ui.GuiSoftwareBox.IsChecked; Save-Settings })
$ui.BackendWsl.Add_Click({ $script:cfg.backend = 'wsl'; Save-Settings; Refresh-All })
$ui.BackendVbox.Add_Click({ $script:cfg.backend = 'virtualbox'; Save-Settings; Refresh-All })
$ui.DistroBox.Add_SelectionChanged({ if ($ui.DistroBox.SelectedItem) { $script:cfg.distro = [string]$ui.DistroBox.SelectedItem; Save-Settings; Refresh-All } })
$ui.VmBox.Add_SelectionChanged({ if ($ui.VmBox.SelectedItem) { $script:cfg.vm = [string]$ui.VmBox.SelectedItem; Save-Settings; Refresh-All } })

if ($script:cfg.advanced -or $Page -eq 'modelo') {
    $ui.AdvancedToggle.IsChecked = $true; $script:cfg.advanced = $true
    $ui.AdvancedTabs.Visibility = 'Visible'; $ui.PreviewCard.Visibility = 'Collapsed'
    if ($Page -eq 'modelo') { $ui.ModelTab.IsSelected = $true }
}

# Every ten seconds, and faster while a model downloads.
$poll = New-Object System.Windows.Threading.DispatcherTimer
$poll.Interval = [TimeSpan]::FromSeconds(10)
$poll.Add_Tick({ if (-not $script:busy) { try { Refresh-All } catch { Add-Log "Error: $($_.Exception.Message)" } } })
$poll.Start()

# The tray: the window can be closed to it while the node keeps running. Its icon is the one
# the installer drew beside this script, else Windows' own.
$tray = New-Object System.Windows.Forms.NotifyIcon
$iconFile = Join-Path $PSScriptRoot 'ArchieOS.ico'
$tray.Icon = if (Test-Path $iconFile) { New-Object System.Drawing.Icon($iconFile, 32, 32) } else { [System.Drawing.SystemIcons]::Application }
if (Test-Path $iconFile) { $window.Icon = [System.Windows.Media.Imaging.BitmapFrame]::Create([Uri]$iconFile) }
$tray.Text = 'ArchieOS'
$tray.Visible = $true
$menu = New-Object System.Windows.Forms.ContextMenuStrip
[void]$menu.Items.Add('Abrir ArchieOS', $null, { $window.Show(); $window.WindowState = 'Normal'; $window.Activate() })
[void]$menu.Items.Add('Abrir GUI', $null, { Open-Gui })
[void]$menu.Items.Add('Reiniciar el nodo', $null, { Start-Node 'restart' })
[void]$menu.Items.Add('Detener el nodo', $null, { Stop-Node })
$startupItem = New-Object System.Windows.Forms.ToolStripMenuItem('Iniciar con Windows')
$startupItem.CheckOnClick = $true
$startupItem.Add_Click({ Set-Autostart $startupItem.Checked; $ui.AutostartBox.IsChecked = $startupItem.Checked; Update-View })
$menu.Add_Opening({ $startupItem.Checked = Test-Path $startupLink })
[void]$menu.Items.Add($startupItem)
[void]$menu.Items.Add('Salir del lanzador', $null, { $script:quit = $true; $window.Close() })
$tray.ContextMenuStrip = $menu
$tray.Add_DoubleClick({ $window.Show(); $window.WindowState = 'Normal'; $window.Activate() })
$script:quit = $false
$window.Add_Closing({ param($s, $e)
    if (-not $script:quit -and (Get-NodeState) -ne 'stopped') {
        # Closing hides it: the node keeps running and the tray brings the window back.
        $e.Cancel = $true; $window.Hide()
        $tray.ShowBalloonTip(3000, 'ArchieOS', 'Sigue en marcha. El icono de la bandeja lo vuelve a abrir.', 'Info')
    }
})
$window.Add_Closed({ $tray.Visible = $false; $tray.Dispose(); $mutex.ReleaseMutex() })

# At sign-in the window says what is about to happen and waits a few seconds for a "no":
# not now, or not at sign-in any more. Then it starts the node and steps aside.
function Start-AtSignIn {
    $script:countdown.Stop()
    $ui.AutostartBanner.Visibility = 'Collapsed'
    $window.WindowState = 'Minimized'
    Add-Log 'Inicio con Windows: levantando el nodo'
    if ($script:cfg.backend -eq 'virtualbox' -or -not (Test-DistroRunning $script:cfg.distro) -or (Get-NodeState) -ne 'running') { Start-Node 'up' }
    else { After-Start }
}
$script:countdown = New-Object System.Windows.Threading.DispatcherTimer
$script:countdown.Interval = [TimeSpan]::FromSeconds(1)
$script:countdown.Add_Tick({
    $script:left--
    if ($script:left -le 0) { Start-AtSignIn; return }
    $ui.AutostartText.Text = "ArchieOS se inicia en $($script:left) s"
})
$ui.AutostartNow.Add_Click({ Start-AtSignIn })
$ui.AutostartSkip.Add_Click({
    $script:countdown.Stop(); $ui.AutostartBanner.Visibility = 'Collapsed'
    Add-Log 'Inicio con Windows: no se inició esta vez'
    Refresh-All
})
$ui.AutostartNever.Add_Click({
    $script:countdown.Stop(); $ui.AutostartBanner.Visibility = 'Collapsed'
    Set-Autostart $false; $ui.AutostartBox.IsChecked = $false
    Add-Log 'Inicio con Windows desactivado. Se vuelve a activar en el panel o en Avanzado → Inicio.'
    Update-View; Refresh-All
})

$window.Add_ContentRendered({
    Update-View
    if ($Autostart) {
        $script:left = [int]$script:cfg.autostart_delay_s
        if ($script:left -le 0) { Start-AtSignIn; return }
        $ui.AutostartText.Text = "ArchieOS se inicia en $($script:left) s"
        $ui.AutostartBanner.Visibility = 'Visible'
        $window.Activate()
        $script:countdown.Start()
    }
    Refresh-All
})
[void]$window.ShowDialog()

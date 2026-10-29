# The ArchieOS installer for Windows, with a window. It reads the machine (GPU, memory,
# drives, WSL, VirtualBox), proposes how ArchieOS fits it best, lets the person change any
# of it, and installs: in WSL, with the GPU given to Linux directly, or in a VirtualBox
# machine, with the local model served from Windows' GPU. Then the launcher, with the start
# at sign-in as chosen.
#
#   Install-ArchieOS.cmd                      double-click (no console window)
#   Install-ArchieOS.ps1 [-Source DIR] [-DryRun]
#
# -Source is the folder with the release (ArchieOS-Operator.wsl or its parts, ArchieOS.ova,
# SHA256SUMS, local-models.tsv): this one by default. -DryRun shows what it would do and
# changes nothing. Windows PowerShell 5.1 and WPF, both part of Windows: nothing to install.
param(
    [string]$Source = $PSScriptRoot,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationFramework, PresentationCore, WindowsBase
$here = $PSScriptRoot
. (Join-Path $here 'ArchieOS-Machine.ps1')
. (Join-Path $here 'ArchieOS-HostModel.ps1')

function First-Existing([string[]]$paths) { $paths | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1 }
$catalogPath = First-Existing @((Join-Path $Source 'local-models.tsv'), (Join-Path $here 'local-models.tsv'),
    (Join-Path $here '..\wsl-operator\rootfs\usr\share\archie\local-models.tsv'))
if (-not $catalogPath) { [System.Windows.MessageBox]::Show('Falta local-models.tsv junto al instalador.', 'ArchieOS') | Out-Null; exit 1 }
$catalog = @(Read-ArchieCatalog $catalogPath)
$operatorInstaller = First-Existing @((Join-Path $Source 'install-archie-operator.ps1'), (Join-Path $here '..\..\scripts\wsl\install-operator.ps1'))
$wslImage = (Test-Path (Join-Path $Source 'ArchieOS-Operator.wsl')) -or [bool](Get-ChildItem $Source -Filter 'ArchieOS-Operator.wsl.0*' -ErrorAction SilentlyContinue)
$ova = Join-Path $Source 'ArchieOS.ova'
$vmImage = Test-Path $ova
$machine = Get-ArchieMachine
$plan = Get-ArchieRecommendation $machine $catalog

[xml]$xaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        Title="Instalar ArchieOS" Width="900" Height="680" MinWidth="760" MinHeight="560"
        Background="#0D1117" Foreground="#E6EDF3" FontFamily="Segoe UI" FontSize="13" WindowStartupLocation="CenterScreen">
  <Window.Resources>
    <Style TargetType="Button">
      <Setter Property="Background" Value="#21262D"/><Setter Property="Foreground" Value="#E6EDF3"/>
      <Setter Property="BorderBrush" Value="#30363D"/><Setter Property="Padding" Value="16,8"/><Setter Property="Margin" Value="8,0,0,0"/>
      <Setter Property="Template"><Setter.Value>
        <ControlTemplate TargetType="Button">
          <Border x:Name="Box" Background="{TemplateBinding Background}" BorderBrush="{TemplateBinding BorderBrush}" BorderThickness="1" CornerRadius="6" Padding="{TemplateBinding Padding}">
            <ContentPresenter HorizontalAlignment="Center" VerticalAlignment="Center"/>
          </Border>
          <ControlTemplate.Triggers>
            <Trigger Property="IsMouseOver" Value="True"><Setter TargetName="Box" Property="Opacity" Value="0.85"/></Trigger>
            <Trigger Property="IsEnabled" Value="False"><Setter TargetName="Box" Property="Opacity" Value="0.4"/></Trigger>
          </ControlTemplate.Triggers>
        </ControlTemplate>
      </Setter.Value></Setter>
    </Style>
    <Style TargetType="TextBlock"><Setter Property="TextWrapping" Value="Wrap"/></Style>
    <Style TargetType="CheckBox"><Setter Property="Foreground" Value="#E6EDF3"/><Setter Property="Margin" Value="0,4"/></Style>
    <Style TargetType="RadioButton"><Setter Property="Foreground" Value="#E6EDF3"/><Setter Property="Margin" Value="0,4"/></Style>
    <Style x:Key="Card" TargetType="Border">
      <Setter Property="Background" Value="#161B22"/><Setter Property="BorderBrush" Value="#30363D"/><Setter Property="BorderThickness" Value="1"/>
      <Setter Property="CornerRadius" Value="10"/><Setter Property="Padding" Value="16"/><Setter Property="Margin" Value="0,0,0,12"/>
    </Style>
    <Style x:Key="Label" TargetType="TextBlock">
      <Setter Property="Foreground" Value="#8B949E"/><Setter Property="FontSize" Value="11"/><Setter Property="FontWeight" Value="SemiBold"/><Setter Property="Margin" Value="0,0,0,8"/>
    </Style>
  </Window.Resources>
  <Grid Margin="22">
    <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
    <StackPanel Grid.Row="0" Margin="0,0,0,14">
      <TextBlock Text="Instalar ArchieOS" FontSize="24" FontWeight="SemiBold"/>
      <TextBlock x:Name="Step" Foreground="#8B949E" Margin="0,4,0,0"/>
    </StackPanel>

    <ScrollViewer Grid.Row="1" x:Name="PageMachine" VerticalScrollBarVisibility="Auto">
      <StackPanel>
        <Border Style="{StaticResource Card}"><StackPanel>
          <TextBlock Style="{StaticResource Label}" Text="TU EQUIPO"/>
          <Grid x:Name="MachineGrid"><Grid.ColumnDefinitions><ColumnDefinition Width="160"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions></Grid>
        </StackPanel></Border>
        <Border Style="{StaticResource Card}" BorderBrush="#388BFD"><StackPanel>
          <TextBlock Style="{StaticResource Label}" Text="LO QUE RECOMIENDO"/>
          <TextBlock x:Name="PlanText"/>
        </StackPanel></Border>
      </StackPanel>
    </ScrollViewer>

    <ScrollViewer Grid.Row="1" x:Name="PageOptions" Visibility="Collapsed" VerticalScrollBarVisibility="Auto">
      <StackPanel>
        <Border Style="{StaticResource Card}"><StackPanel>
          <TextBlock Style="{StaticResource Label}" Text="DÓNDE CORRE ARCHIEOS"/>
          <RadioButton x:Name="BackendWsl" GroupName="backend"/>
          <TextBlock x:Name="WslNote" Foreground="#8B949E" FontSize="12" Margin="22,0,0,8"/>
          <RadioButton x:Name="BackendVbox" GroupName="backend"/>
          <TextBlock x:Name="VboxNote" Foreground="#8B949E" FontSize="12" Margin="22,0,0,0"/>
        </StackPanel></Border>
        <Border Style="{StaticResource Card}"><Grid>
          <Grid.ColumnDefinitions><ColumnDefinition Width="190"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
          <Grid.RowDefinitions><RowDefinition/><RowDefinition/><RowDefinition/><RowDefinition/><RowDefinition/></Grid.RowDefinitions>
          <TextBlock Grid.Row="0" Text="Modelo local" VerticalAlignment="Center"/>
          <ComboBox Grid.Row="0" Grid.Column="1" x:Name="ModelBox" Margin="0,4"/>
          <TextBlock Grid.Row="1" Text="Disco" VerticalAlignment="Center"/>
          <ComboBox Grid.Row="1" Grid.Column="1" x:Name="DriveBox" Margin="0,4"/>
          <TextBlock Grid.Row="2" Text="Memoria para ArchieOS" VerticalAlignment="Center"/>
          <ComboBox Grid.Row="2" Grid.Column="1" x:Name="MemoryBox" Margin="0,4" Width="140" HorizontalAlignment="Left"/>
          <TextBlock Grid.Row="3" Text="Núcleos (VirtualBox)" VerticalAlignment="Center"/>
          <ComboBox Grid.Row="3" Grid.Column="1" x:Name="CpuBox" Margin="0,4" Width="140" HorizontalAlignment="Left"/>
          <TextBlock Grid.Row="4" Text="Clave de Nebius (opcional)" VerticalAlignment="Center"/>
          <PasswordBox Grid.Row="4" Grid.Column="1" x:Name="KeyBox" Margin="0,4" Background="#0D1117" Foreground="#E6EDF3" BorderBrush="#30363D" Padding="4"/>
        </Grid></Border>
        <Border Style="{StaticResource Card}"><StackPanel>
          <TextBlock Style="{StaticResource Label}" Text="INICIO"/>
          <CheckBox x:Name="AutostartBox" Content="Iniciar ArchieOS al entrar a Windows"/>
          <StackPanel Orientation="Horizontal" Margin="22,2,0,6">
            <TextBlock Text="Avisar antes de iniciar durante" VerticalAlignment="Center" Margin="0,0,8,0"/>
            <ComboBox x:Name="DelayBox" Width="90"/>
          </StackPanel>
          <CheckBox x:Name="OpenGuiBox" Content="Abrir la interfaz de ArchieOS cuando arranca"/>
          <CheckBox x:Name="VoiceBox" Content="Abrir la voz cuando arranca"/>
          <TextBlock x:Name="SettingsNote" Foreground="#8B949E" FontSize="12" Margin="0,8,0,0"/>
        </StackPanel></Border>
      </StackPanel>
    </ScrollViewer>

    <Grid Grid.Row="1" x:Name="PageInstall" Visibility="Collapsed">
      <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions>
      <ProgressBar x:Name="Progress" Height="6" IsIndeterminate="True" Margin="0,0,0,12" Foreground="#388BFD" Background="#21262D" BorderThickness="0"/>
      <TextBox Grid.Row="1" x:Name="LogBox" IsReadOnly="True" FontFamily="Consolas" FontSize="12" Background="#0D1117" Foreground="#C9D1D9"
               BorderBrush="#30363D" VerticalScrollBarVisibility="Auto" TextWrapping="Wrap"/>
    </Grid>

    <DockPanel Grid.Row="2" Margin="0,14,0,0">
      <TextBlock x:Name="Footer" Foreground="#8B949E" FontSize="11" VerticalAlignment="Center"/>
      <StackPanel Orientation="Horizontal" HorizontalAlignment="Right">
        <Button x:Name="BackButton" Content="Atrás"/>
        <Button x:Name="NextButton" Content="Siguiente" Background="#1F6FEB" BorderBrush="#388BFD"/>
        <Button x:Name="CloseButton" Content="Cerrar"/>
      </StackPanel>
    </DockPanel>
  </Grid>
</Window>
'@
$window = [Windows.Markup.XamlReader]::Load((New-Object System.Xml.XmlNodeReader $xaml))
$ui = @{}
$xaml.SelectNodes('//*[@*[local-name()="Name"]]') | ForEach-Object {
    $n = $_.Attributes | Where-Object { $_.LocalName -eq 'Name' } | Select-Object -First 1 -ExpandProperty Value
    $ui[$n] = $window.FindName($n)
}
$iconFile = First-Existing @((Join-Path $env:LOCALAPPDATA 'ArchieOS\Launcher\ArchieOS.ico'))
if ($iconFile) { $window.Icon = [System.Windows.Media.Imaging.BitmapFrame]::Create([Uri]$iconFile) }
function Brush([string]$hex) { (New-Object System.Windows.Media.BrushConverter).ConvertFromString($hex) }
function GB([double]$bytes) { '{0:N1} GB' -f ($bytes / 1e9) }

# ---- page 1: the machine -----------------------------------------------------------------
$gpuText = if ($machine.gpus.Count) { ($machine.gpus | ForEach-Object { '{0} · {1:N0} GB · driver {2}' -f $_.name, ($_.vram_mb / 1024), $_.driver }) -join "`n" } else { 'ninguna que ArchieOS pueda usar' }
$rows = @(
    @('Windows', "$($machine.windows) (compilación $($machine.build), $($machine.arch))"),
    @('Procesador', "$($machine.cpu) · $($machine.cores) núcleos, $($machine.threads) hilos"),
    @('Memoria', ('{0:N0} GB' -f ($machine.ram_mb / 1024))),
    @('GPU', $gpuText),
    @('Discos', (($machine.drives | ForEach-Object { '{0}: {1:N0} GB libres de {2:N0}' -f $_.letter, $_.free_gb, $_.size_gb }) -join "`n")),
    @('WSL', $(if ($machine.wsl) { 'instalado' } else { 'no instalado (se instala, y Windows pide reiniciar una vez)' })),
    @('VirtualBox', $(if ($machine.virtualbox) { "instalado ($($machine.virtualbox))" } else { 'no instalado' }))
)
$i = 0
foreach ($r in $rows) {
    $ui.MachineGrid.RowDefinitions.Add((New-Object System.Windows.Controls.RowDefinition))
    $k = New-Object System.Windows.Controls.TextBlock; $k.Text = $r[0]; $k.Foreground = Brush '#8B949E'; $k.Margin = '0,4'
    $v = New-Object System.Windows.Controls.TextBlock; $v.Text = $r[1]; $v.Margin = '0,4'
    [System.Windows.Controls.Grid]::SetRow($k, $i); [System.Windows.Controls.Grid]::SetRow($v, $i); [System.Windows.Controls.Grid]::SetColumn($v, 1)
    [void]$ui.MachineGrid.Children.Add($k); [void]$ui.MachineGrid.Children.Add($v); $i++
}
$where = @{ wsl = 'WSL, con la GPU entregada directamente a Linux'; virtualbox = 'una máquina de VirtualBox, con el modelo servido desde la GPU de Windows'; none = 'en ninguna parte todavía' }
$runtimeNames = @{ 'cuda-12.4' = 'NVIDIA CUDA 12.4'; 'hip-radeon' = 'AMD Radeon (HIP)'; 'vulkan' = 'Vulkan'; 'cpu' = 'la CPU' }
$ui.PlanText.Text = ("Instalar ArchieOS en {0}.`nModelo local: {1} ({2}), porque: {3}.`nRuntime: PrismML llama.cpp para {4}.`nDisco: {5}: · memoria para ArchieOS: {6} GB.`nIniciar al entrar a Windows, con {7} s de aviso y un botón para no iniciar." -f `
    $where[$plan.backend], $plan.model_label, (GB $plan.model_bytes), $plan.why, $runtimeNames[$plan.runtime], $plan.drive, $plan.memory_gb, $plan.autostart_delay_s)

# ---- page 2: the options -----------------------------------------------------------------
$ui.BackendWsl.Content = 'WSL (recomendado)'
$ui.BackendVbox.Content = 'VirtualBox'
$wslReady = $plan.wsl_possible -and ($wslImage -or $DryRun) -and $operatorInstaller
$ui.WslNote.Text = if (-not $plan.wsl_possible) { 'Este Windows no corre WSL 2 con la imagen de ArchieOS.' }
    elseif (-not $wslImage) { "No encuentro ArchieOS-Operator.wsl en $Source." }
    else { 'Linux usa la GPU directamente (CUDA en WSL). La interfaz se abre como ventanas de Windows.' }
$ui.VboxNote.Text = if (-not $plan.virtualbox_possible) { 'Este equipo no corre la imagen de VirtualBox de ArchieOS.' }
    elseif (-not $vmImage) { "La imagen para VirtualBox (ArchieOS.ova) no está en $Source todavía." }
    else { 'Todo ArchieOS en una ventana de VirtualBox a pantalla completa. VirtualBox no da la GPU a una máquina: el modelo corre en Windows con la GPU y la máquina lo usa por su red privada.' }
$ui.BackendWsl.IsEnabled = [bool]$wslReady
$ui.BackendVbox.IsEnabled = $plan.virtualbox_possible -and ($vmImage -or $DryRun)
$ui.BackendWsl.IsChecked = $ui.BackendWsl.IsEnabled -and $plan.backend -eq 'wsl'
$ui.BackendVbox.IsChecked = -not $ui.BackendWsl.IsChecked -and $ui.BackendVbox.IsEnabled

$gpu = $machine.gpus | Sort-Object vram_mb -Descending | Select-Object -First 1
foreach ($m in ($catalog | Where-Object { $_.arch -notin $script:ArchieUnknownArches } | Sort-Object bytes)) {
    $fits = if ($gpu -and $gpu.unified) { $m.bytes / 1MB + 8192 -le $machine.ram_mb } elseif ($gpu) { $m.bytes / 1MB + 1500 -le $gpu.vram_mb } else { $m.bytes / 1MB -le $machine.ram_mb / 2 }
    if (-not $fits -and $m.name -ne $plan.model) { continue }
    $item = New-Object System.Windows.Controls.ComboBoxItem
    $item.Content = '{0} · {1} · licencia {2}{3}' -f $m.label, (GB $m.bytes), $m.license, $(if ($m.name -eq $plan.model) { '  (recomendado)' } else { '' })
    $item.Tag = $m.name
    [void]$ui.ModelBox.Items.Add($item)
    if ($m.name -eq $plan.model) { $ui.ModelBox.SelectedItem = $item }
}
$none = New-Object System.Windows.Controls.ComboBoxItem; $none.Content = 'Ninguno: Archie responde con Nebius (necesita la clave)'; $none.Tag = ''
[void]$ui.ModelBox.Items.Add($none)
foreach ($d in $machine.drives) {
    $item = New-Object System.Windows.Controls.ComboBoxItem
    $item.Content = '{0}: · {1:N0} GB libres{2}' -f $d.letter, $d.free_gb, $(if ($d.letter -eq $plan.drive) { '  (recomendado)' } else { '' })
    $item.Tag = $d.letter
    [void]$ui.DriveBox.Items.Add($item)
    if ($d.letter -eq $plan.drive) { $ui.DriveBox.SelectedItem = $item }
}
$ramGb = [math]::Floor($machine.ram_mb / 1024)
for ($g = 4; $g -le [math]::Max(4, $ramGb - 4); $g++) { [void]$ui.MemoryBox.Items.Add("$g GB") }
$ui.MemoryBox.SelectedItem = "$($plan.memory_gb) GB"
for ($c = 2; $c -le [math]::Max(2, $machine.threads - 2); $c++) { [void]$ui.CpuBox.Items.Add("$c") }
$ui.CpuBox.SelectedItem = "$($plan.cpus)"
foreach ($s in 0, 5, 10, 20, 30, 60) { [void]$ui.DelayBox.Items.Add("$s s") }
$ui.DelayBox.SelectedItem = "$($plan.autostart_delay_s) s"
$ui.AutostartBox.IsChecked = $plan.autostart
$ui.OpenGuiBox.IsChecked = $plan.open_gui_on_start
$ui.VoiceBox.IsChecked = $plan.voice_on_start
$settingsFile = Join-Path $env:APPDATA 'ArchieOS\launcher.json'
$ui.SettingsNote.Text = "Todo esto queda en $settingsFile, que el lanzador lee al iniciar y que se puede editar."

# ---- the installation ----------------------------------------------------------------------
# It runs in a runspace of its own, so the window stays responsive; lines come back through
# a queue the window reads.
$sync = [hashtable]::Synchronized(@{ log = New-Object System.Collections.Concurrent.ConcurrentQueue[string]; done = $false; failed = $null })
function Get-Choices {
    [pscustomobject]@{
        backend = if ($ui.BackendVbox.IsChecked) { 'virtualbox' } else { 'wsl' }
        model = [string]$ui.ModelBox.SelectedItem.Tag
        drive = [string]$ui.DriveBox.SelectedItem.Tag
        memory_gb = [int](([string]$ui.MemoryBox.SelectedItem) -replace '\D', '')
        cpus = [int]$ui.CpuBox.SelectedItem
        autostart = [bool]$ui.AutostartBox.IsChecked
        autostart_delay_s = [int](([string]$ui.DelayBox.SelectedItem) -replace '\D', '')
        open_gui_on_start = [bool]$ui.OpenGuiBox.IsChecked
        voice_on_start = [bool]$ui.VoiceBox.IsChecked
        key = $ui.KeyBox.Password
    }
}

$installer = {
    param($choices, $context)
    $ErrorActionPreference = 'Stop'
    $log = { param($line) $sync.log.Enqueue([string]$line) }
    try {
        . (Join-Path $context.here 'ArchieOS-Machine.ps1')
        . (Join-Path $context.here 'ArchieOS-HostModel.ps1')
        $root = "$($choices.drive):\ArchieOS"
        $entry = $context.catalog | Where-Object { $_.name -eq $choices.model } | Select-Object -First 1
        $settings = [ordered]@{
            backend = $choices.backend; distro = 'ArchieOS'; vm = 'ArchieOS'
            open_gui_on_start = $choices.open_gui_on_start; gui_mode = 'shell'; gui_software = $false
            voice_on_start = $choices.voice_on_start; voice_always = $true; advanced = $false
            autostart_delay_s = $choices.autostart_delay_s; show_all_models = $false
            _ayuda = [ordered]@{
                backend = 'wsl o virtualbox: dónde corre ArchieOS'
                autostart_delay_s = 'segundos de aviso, con un botón para no iniciar, al entrar a Windows (0: sin aviso)'
                open_gui_on_start = 'abrir la interfaz cuando el nodo arranca'
                voice_on_start = 'abrir la voz cuando el nodo arranca'
                inicio_con_windows = 'se activa o desactiva en el lanzador (el acceso directo de la carpeta Inicio)'
            }
        }
        $steps = New-Object System.Collections.ArrayList
        if ($choices.backend -eq 'wsl') {
            if (-not $context.machine.wsl) { [void]$steps.Add('Instalar WSL (permiso de administrador; luego reiniciar Windows y volver a abrir este instalador)') }
            [void]$steps.Add("Memoria de WSL: $($choices.memory_gb) GB en %USERPROFILE%\.wslconfig")
            [void]$steps.Add("Importar ArchieOS en $root\WSL, preparar la voz, el runtime de la GPU y $(if ($entry) { $entry.label } else { 'ningún modelo local' })")
        } else {
            if (-not $context.machine.virtualbox) { [void]$steps.Add('Instalar VirtualBox (permiso de administrador)') }
            if ($entry) { [void]$steps.Add("Runtime $($context.plan.runtime) y $($entry.label) en $root\models, servidos desde Windows") }
            [void]$steps.Add("Importar ArchieOS.ova en $root\VirtualBox con $($choices.cpus) núcleos y $($choices.memory_gb) GB")
        }
        if ($choices.key) { [void]$steps.Add('Guardar la clave de Nebius dentro de ArchieOS') }
        [void]$steps.Add("Instalar el lanzador; inicio con Windows: $(if ($choices.autostart) { "sí, con $($choices.autostart_delay_s) s de aviso" } else { 'no' })")
        [void]$steps.Add("Escribir $($context.settingsFile)")
        & $log 'Pasos:'
        $steps | ForEach-Object { & $log "  · $_" }
        if ($context.dryRun) { & $log ''; & $log 'Prueba (-DryRun): no se cambió nada.'; return }
        & $log ''

        if ($choices.backend -eq 'wsl') {
            if (-not $context.machine.wsl) {
                & $log 'Instalando WSL: Windows pedirá permiso de administrador.'
                $p = Start-Process wsl.exe -Verb RunAs -ArgumentList '--install', '--no-distribution' -Wait -PassThru
                if ($p.ExitCode -ne 0) { throw 'WSL no se instaló' }
                # After the restart Windows opens this installer again by itself, once.
                Set-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce' 'ArchieOS-Install' ('wscript.exe "{0}"' -f (Join-Path $context.here 'Install-ArchieOS.vbs'))
                & $log 'WSL está instalado. Reinicia Windows: este instalador se abre solo al volver.'
                return
            }
            # The memory WSL may take, kept with whatever else the person set there.
            $wslconfig = Join-Path $env:USERPROFILE '.wslconfig'
            $lines = if (Test-Path $wslconfig) { @(Get-Content $wslconfig) } else { @('[wsl2]') }
            if (-not ($lines -match '^\s*\[wsl2\]')) { $lines = @('[wsl2]') + $lines }
            $lines = @($lines | Where-Object { $_ -notmatch '^\s*memory\s*=' })
            $at = [array]::IndexOf($lines, @($lines -match '^\s*\[wsl2\]')[0])
            $rest = if ($at + 1 -lt $lines.Count) { @($lines[($at + 1)..($lines.Count - 1)]) } else { @() }
            $lines = @($lines[0..$at]) + @("memory=$($choices.memory_gb)GB") + $rest
            # Without a byte order mark: WSL reads the file as plain text.
            [System.IO.File]::WriteAllLines($wslconfig, [string[]]$lines, (New-Object System.Text.UTF8Encoding($false)))
            # It applies the next time WSL starts; other distributions running now are left alone.
            & $log "WSL tendrá $($choices.memory_gb) GB la próxima vez que arranque."
            $options = @{ Name = 'ArchieOS'; InstallDir = "$root\WSL"; Source = $context.source; Unattended = $true; NoLauncher = $true }
            if ($entry) { $options.Model = $entry.name }
            & $context.operatorInstaller @options *>&1 | ForEach-Object { & $log "$_" }
            if ($choices.key) {
                & $log 'Guardando la clave de Nebius'
                $psi = New-Object System.Diagnostics.ProcessStartInfo 'wsl.exe', '-d ArchieOS -u archie -- archie-setup key'
                $psi.UseShellExecute = $false; $psi.RedirectStandardInput = $true; $psi.CreateNoWindow = $true
                $p = [System.Diagnostics.Process]::Start($psi)
                $p.StandardInput.WriteLine($choices.key); $p.StandardInput.Close(); $p.WaitForExit()
            }
        } else {
            $vbox = Join-Path $env:ProgramFiles 'Oracle\VirtualBox\VBoxManage.exe'
            if (-not (Test-Path $vbox)) {
                & $log 'Descargando VirtualBox'
                $latest = (Invoke-WebRequest -UseBasicParsing 'https://download.virtualbox.org/virtualbox/LATEST-STABLE.TXT').Content.Trim()
                $sums = (Invoke-WebRequest -UseBasicParsing "https://download.virtualbox.org/virtualbox/$latest/SHA256SUMS").Content -split "`n"
                $line = $sums | Where-Object { $_ -match "VirtualBox-$([regex]::Escape($latest))-\d+-Win\.exe" } | Select-Object -First 1
                if (-not $line) { throw "No encuentro el instalador de VirtualBox $latest para Windows" }
                $sha, $file = ($line.Trim() -split '\s+\*?')
                $setup = Join-Path $env:LOCALAPPDATA "ArchieOS\downloads\$file"
                Save-Verified "https://download.virtualbox.org/virtualbox/$latest/$file" $setup $sha $log
                & $log 'Instalando VirtualBox: Windows pedirá permiso de administrador.'
                $p = Start-Process $setup -Verb RunAs -ArgumentList '--silent', '--ignore-reboot' -Wait -PassThru
                if ($p.ExitCode -ne 0 -or -not (Test-Path $vbox)) { throw 'VirtualBox no se instaló' }
            }
            $hostModel = $null
            if ($entry) {
                $runtimeDir = Install-HostRuntime $context.plan.runtime $log
                $path = Install-HostModel $entry "$root\models" $log
                $hostModel = [ordered]@{
                    name = $entry.name; alias = $entry.alias; port = $entry.port; reasoning = $entry.reasoning
                    path = $path; runtime = $context.plan.runtime; runtime_dir = $runtimeDir
                }
                $settings.host_model = $hostModel
            }
            $ovaSums = Join-Path $context.source 'SHA256SUMS'
            if (Test-Path $ovaSums) {
                $line = Get-Content $ovaSums | Where-Object { $_ -match 'ArchieOS\.ova$' } | Select-Object -First 1
                if ($line -and (Get-FileHash $context.ova -Algorithm SHA256).Hash -ne ($line -split '\s+')[0].ToUpper()) { throw 'ArchieOS.ova está dañado o incompleto' }
            }
            New-ArchieVm $context.ova 'ArchieOS' $choices.cpus $choices.memory_gb "$root\VirtualBox" $hostModel $log
        }

        & $log 'Instalando el lanzador'
        $launcherOptions = @{ Distro = 'ArchieOS' }
        if (-not $choices.autostart) { $launcherOptions.NoAutostart = $true }
        & (Join-Path $context.here 'Install-ArchieOS-Launcher.ps1') @launcherOptions *>&1 | ForEach-Object { & $log "$_" }
        New-Item -ItemType Directory -Force (Split-Path $context.settingsFile) | Out-Null
        $saved = if (Test-Path $context.settingsFile) { Get-Content $context.settingsFile -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
        if ($saved) { foreach ($p in $saved.PSObject.Properties) { if (-not $settings.Contains($p.Name)) { $settings[$p.Name] = $p.Value } } }
        ($settings | ConvertTo-Json -Depth 4) | Set-Content $context.settingsFile -Encoding UTF8
        & $log "Ajustes en $($context.settingsFile)"
        & $log ''
        & $log 'Listo. El icono ArchieOS del escritorio lo abre.'
    } catch {
        $sync.failed = $_.Exception.Message
        & $log "Error: $($_.Exception.Message)"
    } finally {
        $sync.done = $true
    }
}

function Start-Install {
    $choices = Get-Choices
    if ($choices.backend -eq 'wsl' -and -not $choices.model -and -not $choices.key) {
        [System.Windows.MessageBox]::Show('Sin modelo local, Archie necesita la clave de Nebius para responder.', 'ArchieOS') | Out-Null; return
    }
    Show-Page 3
    $context = @{
        here = $here; source = $Source; catalog = $catalog; machine = $machine; plan = $plan; ova = $ova
        operatorInstaller = $operatorInstaller; settingsFile = $settingsFile; dryRun = [bool]$DryRun
    }
    $runspace = [runspacefactory]::CreateRunspace()
    $runspace.ApartmentState = 'STA'; $runspace.Open()
    $runspace.SessionStateProxy.SetVariable('sync', $sync)
    $script:job = [powershell]::Create()
    $script:job.Runspace = $runspace
    [void]$script:job.AddScript($installer).AddArgument($choices).AddArgument($context)
    [void]$script:job.BeginInvoke()
    $timer = New-Object System.Windows.Threading.DispatcherTimer
    $timer.Interval = [TimeSpan]::FromMilliseconds(300)
    $timer.Add_Tick({
        $line = $null
        while ($sync.log.TryDequeue([ref]$line)) { $ui.LogBox.AppendText("$line`r`n") }
        $ui.LogBox.ScrollToEnd()
        if ($sync.done) {
            $this.Stop()
            $ui.Progress.IsIndeterminate = $false; $ui.Progress.Value = 100
            $ui.Progress.Foreground = Brush $(if ($sync.failed) { '#F85149' } else { '#3FB950' })
            $ui.Step.Text = if ($sync.failed) { 'No terminó: el registro dice por qué. Se puede volver a ejecutar.' } elseif ($DryRun) { 'Prueba terminada: no se cambió nada.' } else { 'ArchieOS está instalado.' }
            $ui.CloseButton.IsEnabled = $true
            $ui.NextButton.Visibility = 'Collapsed'
            if (-not $sync.failed -and -not $DryRun) {
                $ui.NextButton.Content = 'Abrir ArchieOS'; $ui.NextButton.Visibility = 'Visible'; $ui.NextButton.IsEnabled = $true
            }
        }
    })
    $timer.Start()
}

$script:page = 1
function Show-Page([int]$n) {
    $script:page = $n
    $ui.PageMachine.Visibility = if ($n -eq 1) { 'Visible' } else { 'Collapsed' }
    $ui.PageOptions.Visibility = if ($n -eq 2) { 'Visible' } else { 'Collapsed' }
    $ui.PageInstall.Visibility = if ($n -eq 3) { 'Visible' } else { 'Collapsed' }
    $ui.BackButton.Visibility = if ($n -eq 2) { 'Visible' } else { 'Collapsed' }
    $ui.Step.Text = @{ 1 = 'Paso 1 de 3: lo que encontré en este equipo'; 2 = 'Paso 2 de 3: cómo instalarlo (todo se puede cambiar)'; 3 = 'Paso 3 de 3: instalando' }[$n]
    $ui.NextButton.Content = @{ 1 = 'Siguiente'; 2 = $(if ($DryRun) { 'Probar' } else { 'Instalar' }); 3 = 'Instalando…' }[$n]
    $ui.NextButton.IsEnabled = $n -ne 3 -and ($n -ne 2 -or $ui.BackendWsl.IsChecked -or $ui.BackendVbox.IsChecked)
    $ui.CloseButton.IsEnabled = $n -ne 3
}
$ui.NextButton.Add_Click({
    if ($ui.NextButton.Content -eq 'Abrir ArchieOS') {
        Start-Process (Join-Path $env:WINDIR 'System32\wscript.exe') -ArgumentList ('"{0}"' -f (Join-Path $env:LOCALAPPDATA 'ArchieOS\Launcher\ArchieOS.vbs'))
        $window.Close(); return
    }
    if ($script:page -eq 1) { Show-Page 2 } elseif ($script:page -eq 2) { Start-Install }
})
$ui.BackButton.Add_Click({ Show-Page 1 })
$ui.CloseButton.Add_Click({ $window.Close() })
$ui.BackendWsl.Add_Checked({ $ui.CpuBox.IsEnabled = $false; Show-Page 2 })
$ui.BackendVbox.Add_Checked({ $ui.CpuBox.IsEnabled = $true; Show-Page 2 })
$ui.CpuBox.IsEnabled = [bool]$ui.BackendVbox.IsChecked
$ui.Footer.Text = "Origen: $Source$(if ($DryRun) { ' · prueba: no cambia nada' })"
if ($plan.backend -eq 'none') { $ui.PlanText.Text = "No hay una forma de instalar ArchieOS en este equipo todavía: $($plan.why)." }
Show-Page 1
[void]$window.ShowDialog()

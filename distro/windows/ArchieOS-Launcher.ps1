# ArchieOS on Windows: starts, restarts and stops the node in WSL (or its VirtualBox VM),
# shows how it is set up, opens the GUI and the voice, and changes the local model.
#
#   ArchieOS-Launcher.ps1              the window
#   ArchieOS-Launcher.ps1 -Autostart   at Windows sign-in: starts the node, then opens the
#                                      GUI if so set, with the window minimized
#   ArchieOS-Launcher.ps1 -Page modelo opens on the model wizard (the installer does this)
#   ArchieOS-Launcher.ps1 -Page instalar opens on Instalar ArchieOS (ArchieOS-Installer.ps1)
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
        # Where Instalar ArchieOS put it (downloads, the WSL disk, its state and logs).
        install_root = ''; install_distro = ''
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
# Instalar ArchieOS: the installer's functions (its state, manifest and credentials). Its long
# steps run as a process of their own, so a download does not freeze this window.
$installer = Join-Path $PSScriptRoot 'ArchieOS-Installer.ps1'
if (Test-Path $installer) { . $installer }

# ---- the window -------------------------------------------------------------------------
[xml]$xaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        Title="ArchieOS" Width="980" Height="700" MinWidth="760" MinHeight="560"
        Background="#12131A" Foreground="#F2F1EE" FontFamily="Segoe UI" FontSize="13"
        WindowStartupLocation="CenterScreen">
  <Window.Resources>
    <!-- ArchieOS Launcher: tema base (tokens del sistema de diseño ArchieOS Launcher). Un solo tema, oscuro y opaco. -->
    <Style x:Key="ArchieFocus">
      <Setter Property="Control.Template">
        <Setter.Value>
          <ControlTemplate>
            <Rectangle Margin="-3" RadiusX="10" RadiusY="10" StrokeThickness="2" Stroke="#6FC8F5" IsHitTestVisible="False"/>
          </ControlTemplate>
        </Setter.Value>
      </Setter>
    </Style>
    <Style TargetType="Button">
      <Setter Property="Background" Value="#2A2B32"/>
      <Setter Property="Foreground" Value="#F2F1EE"/>
      <Setter Property="BorderBrush" Value="#2C2D36"/>
      <Setter Property="Padding" Value="14,7"/>
      <Setter Property="Margin" Value="0,0,8,0"/>
      <Setter Property="Cursor" Value="Hand"/>
      <Setter Property="FocusVisualStyle" Value="{StaticResource ArchieFocus}"/>
      <Setter Property="Template">
        <Setter.Value>
          <ControlTemplate TargetType="Button">
            <Grid>
              <Border x:Name="Box" Background="{TemplateBinding Background}" BorderBrush="{TemplateBinding BorderBrush}"
                      BorderThickness="1" CornerRadius="8"/>
              <Border x:Name="Over" Background="#00FFFFFF" CornerRadius="8" IsHitTestVisible="False"/>
              <Rectangle x:Name="Dash" Visibility="Collapsed" RadiusX="8" RadiusY="8" StrokeThickness="1"
                         Stroke="#998D8C9C" StrokeDashArray="3 3" IsHitTestVisible="False"/>
              <ContentPresenter x:Name="Content" Margin="{TemplateBinding Padding}" HorizontalAlignment="Center" VerticalAlignment="Center"
                                TextElement.Foreground="{TemplateBinding Foreground}"/>
            </Grid>
            <ControlTemplate.Triggers>
              <Trigger Property="IsMouseOver" Value="True"><Setter TargetName="Over" Property="Background" Value="#14FFFFFF"/></Trigger>
              <Trigger Property="IsPressed" Value="True"><Setter TargetName="Over" Property="Background" Value="#26FFFFFF"/></Trigger>
              <Trigger Property="IsEnabled" Value="False">
                <Setter TargetName="Box" Property="Background" Value="Transparent"/>
                <Setter TargetName="Box" Property="BorderBrush" Value="Transparent"/>
                <Setter TargetName="Over" Property="Background" Value="#00FFFFFF"/>
                <Setter TargetName="Dash" Property="Visibility" Value="Visible"/>
                <Setter TargetName="Content" Property="TextElement.Foreground" Value="#5E5D6B"/>
              </Trigger>
            </ControlTemplate.Triggers>
          </ControlTemplate>
        </Setter.Value>
      </Setter>
    </Style>
    <Style TargetType="CheckBox">
      <Setter Property="Foreground" Value="#F2F1EE"/><Setter Property="Margin" Value="0,4"/>
      <Setter Property="FocusVisualStyle" Value="{StaticResource ArchieFocus}"/>
      <Setter Property="Template">
        <Setter.Value>
          <ControlTemplate TargetType="CheckBox">
            <Grid Background="Transparent">
              <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
              <Border x:Name="Box" Width="16" Height="16" CornerRadius="4" BorderThickness="1" BorderBrush="#8D8C9C"
                      Background="Transparent" VerticalAlignment="Top" Margin="0,1,8,0">
                <Path x:Name="Tick" Data="M3.5,8.5 L6.5,11.5 L12.5,4.5" Stroke="#06151D" StrokeThickness="1.8"
                      StrokeStartLineCap="Round" StrokeEndLineCap="Round" StrokeLineJoin="Round" Visibility="Collapsed"/>
              </Border>
              <ContentPresenter Grid.Column="1" VerticalAlignment="Center" TextElement.Foreground="{TemplateBinding Foreground}"/>
            </Grid>
            <ControlTemplate.Triggers>
              <Trigger Property="IsChecked" Value="True">
                <Setter TargetName="Box" Property="Background" Value="#6FC8F5"/>
                <Setter TargetName="Box" Property="BorderBrush" Value="#6FC8F5"/>
                <Setter TargetName="Tick" Property="Visibility" Value="Visible"/>
              </Trigger>
              <Trigger Property="IsEnabled" Value="False"><Setter Property="Opacity" Value="0.5"/></Trigger>
            </ControlTemplate.Triggers>
          </ControlTemplate>
        </Setter.Value>
      </Setter>
    </Style>
    <Style TargetType="RadioButton">
      <Setter Property="Foreground" Value="#F2F1EE"/><Setter Property="Margin" Value="0,4,16,4"/>
      <Setter Property="FocusVisualStyle" Value="{StaticResource ArchieFocus}"/>
      <Setter Property="Template">
        <Setter.Value>
          <ControlTemplate TargetType="RadioButton">
            <Grid Background="Transparent">
              <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
              <Ellipse x:Name="Ring" Width="16" Height="16" StrokeThickness="1" Stroke="#8D8C9C" Fill="Transparent" VerticalAlignment="Center" Margin="0,0,8,0"/>
              <Ellipse x:Name="Dot" Width="8" Height="8" Fill="#6FC8F5" Visibility="Collapsed" Margin="0,0,8,0"/>
              <ContentPresenter Grid.Column="1" VerticalAlignment="Center" TextElement.Foreground="{TemplateBinding Foreground}"/>
            </Grid>
            <ControlTemplate.Triggers>
              <Trigger Property="IsChecked" Value="True">
                <Setter TargetName="Ring" Property="Stroke" Value="#6FC8F5"/>
                <Setter TargetName="Dot" Property="Visibility" Value="Visible"/>
              </Trigger>
              <Trigger Property="IsEnabled" Value="False"><Setter Property="Opacity" Value="0.5"/></Trigger>
            </ControlTemplate.Triggers>
          </ControlTemplate>
        </Setter.Value>
      </Setter>
    </Style>
    <Style TargetType="TextBox">
      <Setter Property="Background" Value="#22232A"/><Setter Property="Foreground" Value="#F2F1EE"/>
      <Setter Property="BorderBrush" Value="#2C2D36"/><Setter Property="CaretBrush" Value="#F2F1EE"/>
      <Setter Property="SelectionBrush" Value="#6FC8F5"/>
    </Style>
    <Style TargetType="TextBlock"><Setter Property="TextWrapping" Value="Wrap"/></Style>
    <Style TargetType="ProgressBar">
      <Setter Property="Height" Value="6"/><Setter Property="Minimum" Value="0"/><Setter Property="Maximum" Value="1"/>
      <Setter Property="Background" Value="#22232A"/><Setter Property="Foreground" Value="#6FC8F5"/>
      <Setter Property="BorderThickness" Value="0"/>
    </Style>
    <Style x:Key="Card" TargetType="Border">
      <Setter Property="Background" Value="#16171F"/><Setter Property="BorderBrush" Value="#2C2D36"/>
      <Setter Property="BorderThickness" Value="1"/><Setter Property="CornerRadius" Value="10"/>
      <Setter Property="Padding" Value="16"/><Setter Property="Margin" Value="0,0,0,12"/>
    </Style>
    <Style x:Key="Label" TargetType="TextBlock">
      <Setter Property="Foreground" Value="#8D8C9C"/><Setter Property="FontSize" Value="10"/>
      <Setter Property="FontWeight" Value="SemiBold"/><Setter Property="Margin" Value="0,0,0,8"/>
    </Style>
  </Window.Resources>
  <Grid Margin="20">
    <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
    <DockPanel Grid.Row="0" Margin="0,0,0,16">
      <StackPanel Orientation="Horizontal" DockPanel.Dock="Left" VerticalAlignment="Center">
        <TextBlock Text="ArchieOS" FontSize="24" FontWeight="SemiBold"/>
        <Border x:Name="StatePill" CornerRadius="11" BorderThickness="1" Padding="10,3" Margin="14,4,0,0" Background="#2C2D36" VerticalAlignment="Center">
          <TextBlock x:Name="StateText" Text="Leyendo…" FontSize="12" FontWeight="SemiBold"/>
        </Border>
      </StackPanel>
      <CheckBox x:Name="AdvancedToggle" Content="Avanzado" DockPanel.Dock="Right" HorizontalAlignment="Right" VerticalAlignment="Center"/>
    </DockPanel>

    <Grid Grid.Row="1">
      <Grid.ColumnDefinitions><ColumnDefinition Width="360"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
      <ScrollViewer Grid.Column="0" VerticalScrollBarVisibility="Auto" Margin="0,0,16,0">
      <StackPanel>
        <Border x:Name="AutostartBanner" Visibility="Collapsed" Background="#16171F" BorderBrush="#6FC8F5"
                BorderThickness="1" CornerRadius="10" Padding="14" Margin="0,0,0,12">
          <StackPanel>
            <TextBlock x:Name="AutostartText" FontWeight="SemiBold" Margin="0,0,0,10"/>
            <WrapPanel>
              <Button x:Name="AutostartNow" Content="Iniciar ahora" Background="#6FC8F5" Foreground="#06151D" BorderBrush="#6FC8F5" Margin="0,0,8,8"/>
              <Button x:Name="AutostartSkip" Content="No iniciar esta vez" Margin="0,0,8,8"/>
              <Button x:Name="AutostartNever" Content="No iniciar con Windows" Margin="0,0,8,8"/>
            </WrapPanel>
          </StackPanel>
        </Border>
        <Border x:Name="InstallCard" Style="{StaticResource Card}">
          <StackPanel>
            <TextBlock Style="{StaticResource Label}" Text="ARCHIEOS"/>
            <TextBlock x:Name="InstallTitle" FontSize="16" FontWeight="SemiBold" Margin="0,0,0,4"/>
            <TextBlock x:Name="InstallSummary" Foreground="#8D8C9C" Margin="0,0,0,10"/>
            <ProgressBar x:Name="InstallBar" Visibility="Collapsed" Margin="0,0,0,6"/>
            <TextBlock x:Name="InstallProgressText" Visibility="Collapsed" Foreground="#B9B8C4" FontSize="12" Margin="0,0,0,10"/>
            <Button x:Name="InstallStart" Content="Iniciar ArchieOS" FontSize="16" FontWeight="SemiBold" Padding="18,12"
                    Background="#6FC8F5" Foreground="#06151D" BorderBrush="#6FC8F5" Margin="0,0,0,10" HorizontalAlignment="Stretch"/>
            <WrapPanel>
              <Button x:Name="InstallOpen" Content="Instalar ArchieOS" Margin="0,0,8,8"/>
              <Button x:Name="InstallStop" Content="Detener" Margin="0,0,8,8"/>
              <Button x:Name="InstallVerify" Content="Verificar/Reparar" Margin="0,0,8,8"/>
              <Button x:Name="InstallCancel" Content="Cancelar" Visibility="Collapsed" Margin="0,0,8,8"/>
              <Button x:Name="InstallLogs" Content="Registros" Margin="0,0,8,8"/>
            </WrapPanel>
          </StackPanel>
        </Border>
        <Border Style="{StaticResource Card}">
          <StackPanel>
            <TextBlock Style="{StaticResource Label}" Text="NODO"/>
            <TextBlock x:Name="StateDetail" Text="" Foreground="#8D8C9C" Margin="0,0,0,14"/>
            <Button x:Name="PrimaryButton" Content="Iniciar" FontSize="16" FontWeight="SemiBold" Padding="18,12"
                    Background="#6FC8F5" Foreground="#06151D" BorderBrush="#6FC8F5" Margin="0,0,0,10" HorizontalAlignment="Stretch"/>
            <WrapPanel>
              <Button x:Name="GuiButton" Content="Abrir GUI" Margin="0,0,8,8"/>
              <Button x:Name="TalkButton" Content="Hablar con Archie" Margin="0,0,8,8"/>
              <Button x:Name="StopButton" Content="Detener" Margin="0,0,8,8"/>
            </WrapPanel>
            <CheckBox x:Name="StartupToggle" Content="Iniciar ArchieOS al entrar a Windows" Margin="0,6,0,0"/>
            <TextBlock x:Name="DiskWarning" Visibility="Collapsed" Foreground="#F0B429" FontSize="12" Margin="0,10,0,6"/>
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
              <TextBlock Margin="0,14,0,0" Foreground="#8D8C9C" FontSize="12"
                Text="Avanzado muestra el asistente del modelo local, el inicio con Windows, el destino (WSL o VirtualBox) y el registro."/>
            </StackPanel>
          </ScrollViewer>
        </Border>

        <TabControl x:Name="AdvancedTabs" Visibility="Collapsed" Background="#16171F" BorderBrush="#2C2D36" Foreground="#F2F1EE">
          <TabControl.Resources>
            <!-- A tab's content takes its text color from the TabControl, black by default. -->
            <Style TargetType="TabItem">
              <Setter Property="Foreground" Value="#F2F1EE"/>
              <Setter Property="Padding" Value="12,5"/>
              <Setter Property="FocusVisualStyle" Value="{StaticResource ArchieFocus}"/>
              <Setter Property="Template">
                <Setter.Value>
                  <ControlTemplate TargetType="TabItem">
                    <Border x:Name="Hdr" Padding="{TemplateBinding Padding}" BorderThickness="0,0,0,2" BorderBrush="Transparent" Background="Transparent" Cursor="Hand">
                      <ContentPresenter x:Name="Hc" ContentSource="Header" TextElement.Foreground="#B9B8C4"/>
                    </Border>
                    <ControlTemplate.Triggers>
                      <Trigger Property="IsMouseOver" Value="True"><Setter TargetName="Hc" Property="TextElement.Foreground" Value="#F2F1EE"/></Trigger>
                      <Trigger Property="IsSelected" Value="True">
                        <Setter TargetName="Hdr" Property="BorderBrush" Value="#6FC8F5"/>
                        <Setter TargetName="Hc" Property="TextElement.Foreground" Value="#F2F1EE"/>
                      </Trigger>
                    </ControlTemplate.Triggers>
                  </ControlTemplate>
                </Setter.Value>
              </Setter>
            </Style>
          </TabControl.Resources>
          <TabItem x:Name="ModelTab" Header="Modelo local">
            <DockPanel Margin="12" TextElement.Foreground="#F2F1EE">
              <TextBlock DockPanel.Dock="Top" x:Name="ModelIntro" Foreground="#8D8C9C" Margin="0,0,0,6"/>
              <CheckBox DockPanel.Dock="Top" x:Name="ShowAllModels" Content="Mostrar también los que no caben en este equipo" Margin="0,0,0,8"/>
              <DockPanel DockPanel.Dock="Bottom" Margin="0,10,0,0">
                <TextBlock Text="Carpeta de modelos:" VerticalAlignment="Center" Margin="0,0,8,0"/>
                <Button x:Name="ModelsDirApply" Content="Cambiar" DockPanel.Dock="Right" Margin="8,0,0,0"/>
                <TextBox x:Name="ModelsDirBox" Background="#22232A" Foreground="#F2F1EE" BorderBrush="#2C2D36" Padding="4"/>
              </DockPanel>
              <ScrollViewer VerticalScrollBarVisibility="Auto"><StackPanel x:Name="ModelList"/></ScrollViewer>
            </DockPanel>
          </TabItem>
          <TabItem x:Name="InstallTab" Header="Instalar">
            <ScrollViewer VerticalScrollBarVisibility="Auto">
              <StackPanel Margin="12" TextElement.Foreground="#F2F1EE">
                <TextBlock Style="{StaticResource Label}" Text="1 · SISTEMA"/>
                <TextBlock x:Name="InstSystem" Margin="0,0,0,14"/>
                <TextBlock Style="{StaticResource Label}" Text="2 · MODO"/>
                <RadioButton x:Name="InstModeWsl" Content="Operator (WSL2)" GroupName="instmode" IsChecked="True"/>
                <TextBlock x:Name="InstModeNote" Foreground="#8D8C9C" FontSize="12" Margin="0,4,0,14"/>
                <TextBlock Style="{StaticResource Label}" Text="3 · CARPETA"/>
                <DockPanel>
                  <Button x:Name="InstBrowse" Content="Elegir…" DockPanel.Dock="Right" Margin="8,0,0,0"/>
                  <TextBox x:Name="InstRoot" Padding="4" VerticalContentAlignment="Center"/>
                </DockPanel>
                <TextBlock x:Name="InstSpace" Foreground="#8D8C9C" FontSize="12" Margin="0,6,0,8"/>
                <StackPanel Orientation="Horizontal" Margin="0,0,0,14">
                  <TextBlock Text="Distribución de WSL:" VerticalAlignment="Center" Margin="0,0,8,0"/>
                  <TextBox x:Name="InstDistro" Width="220" Padding="4"/>
                </StackPanel>
                <TextBlock Style="{StaticResource Label}" Text="4 · ACCESO A LA DESCARGA"/>
                <TextBlock x:Name="InstAuthNote" Foreground="#8D8C9C" FontSize="12" Margin="0,0,0,6"/>
                <DockPanel Margin="0,0,0,14">
                  <Button x:Name="InstTokenForget" Content="Olvidar" DockPanel.Dock="Right" Margin="8,0,0,0"/>
                  <Button x:Name="InstTokenSave" Content="Guardar" DockPanel.Dock="Right" Margin="8,0,0,0"/>
                  <PasswordBox x:Name="InstToken" Background="#22232A" Foreground="#F2F1EE" BorderBrush="#2C2D36" CaretBrush="#F2F1EE" Padding="4"/>
                </DockPanel>
                <TextBlock Style="{StaticResource Label}" Text="5 · REQUISITOS"/>
                <StackPanel x:Name="InstDeps" Margin="0,0,0,8"/>
                <WrapPanel Margin="0,0,0,14">
                  <Button x:Name="InstAnalyze" Content="Analizar" Margin="0,0,8,8"/>
                  <Button x:Name="InstFixDeps" Content="Instalar requisitos" Margin="0,0,8,8"/>
                </WrapPanel>
                <TextBlock Style="{StaticResource Label}" Text="6 · COMPONENTES"/>
                <StackPanel x:Name="InstArtifacts" Margin="0,0,0,8"/>
                <ProgressBar x:Name="InstBar" Visibility="Collapsed" Margin="0,4,0,6"/>
                <TextBlock x:Name="InstProgressText" Foreground="#B9B8C4" FontSize="12" Margin="0,0,0,8"/>
                <WrapPanel>
                  <Button x:Name="InstGo" Content="Instalar ArchieOS" Background="#6FC8F5" Foreground="#06151D" BorderBrush="#6FC8F5" Margin="0,0,8,8"/>
                  <Button x:Name="InstCancel" Content="Cancelar" Margin="0,0,8,8"/>
                </WrapPanel>
              </StackPanel>
            </ScrollViewer>
          </TabItem>
          <TabItem Header="Inicio">
            <StackPanel Margin="12" TextElement.Foreground="#F2F1EE">
              <CheckBox x:Name="AutostartBox" Content="Iniciar ArchieOS al entrar a Windows"/>
              <CheckBox x:Name="OpenGuiBox" Content="Abrir la GUI cuando el nodo está en marcha"/>
              <CheckBox x:Name="VoiceOnStartBox" Content="Abrir la voz al iniciar"/>
              <CheckBox x:Name="VoiceAlwaysBox" Content="La voz responde a todo lo que oye (sin decir «Archie»)"/>
              <TextBlock Text="GUI" Margin="0,14,0,4" Foreground="#8D8C9C"/>
              <StackPanel Orientation="Horizontal">
                <RadioButton x:Name="GuiShell" Content="Escritorio (shell)" GroupName="gui"/>
                <RadioButton x:Name="GuiObservatory" Content="Observatorio" GroupName="gui"/>
              </StackPanel>
              <CheckBox x:Name="GuiSoftwareBox" Content="Dibujar con la CPU (si la GPU está ocupada o la GUI se congela)"/>
              <TextBlock Text="Segundos de aviso antes de iniciar al entrar a Windows" Margin="0,14,0,4" Foreground="#8D8C9C"/>
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
            <StackPanel Margin="12" TextElement.Foreground="#F2F1EE">
              <StackPanel Orientation="Horizontal">
                <RadioButton x:Name="BackendWsl" Content="WSL" GroupName="backend"/>
                <RadioButton x:Name="BackendVbox" Content="VirtualBox" GroupName="backend"/>
              </StackPanel>
              <TextBlock Text="Distribución de WSL" Margin="0,12,0,4" Foreground="#8D8C9C"/>
              <ComboBox x:Name="DistroBox" Width="360" HorizontalAlignment="Left"/>
              <TextBlock Text="Máquina de VirtualBox" Margin="0,12,0,4" Foreground="#8D8C9C"/>
              <ComboBox x:Name="VmBox" Width="360" HorizontalAlignment="Left"/>
              <TextBlock x:Name="VboxNote" Margin="0,8,0,0" Foreground="#8D8C9C"/>
            </StackPanel>
          </TabItem>
          <TabItem Header="Registro">
            <TextBox x:Name="LogBox" Margin="8" IsReadOnly="True" FontFamily="Consolas" FontSize="12"
                     Background="#12131A" Foreground="#B9B8C4" BorderBrush="#2C2D36"
                     VerticalScrollBarVisibility="Auto" TextWrapping="Wrap"/>
          </TabItem>
        </TabControl>
      </Grid>
    </Grid>

    <TextBlock Grid.Row="2" x:Name="Footer" Foreground="#8D8C9C" FontSize="11" Margin="0,8,0,0"/>
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
    # Chip de estado: fondo, borde y texto por estado (solo atención y fallo llevan tinte).
    $chip = @{
        running     = @('#22232A', '#2C2D36', '#47D18B')
        stopped     = @('#22232A', '#2C2D36', '#B9B8C4')
        partial     = @('#1FF0B429', '#5CF0B429', '#F0B429')
        unavailable = @('#21F2626B', '#66F2626B', '#F2626B')
    }
    $busyChip = @('#1F6FE3F5', '#406FE3F5', '#6FE3F5')
    $busyText = @{ start = 'Iniciando…'; restart = 'Reiniciando…'; stop = 'Deteniendo…'; model = 'Cambiando de modelo…'; gui = 'Abriendo la GUI…'; compact = 'Liberando espacio…' }
    if ($script:busy) { $ui.StateText.Text = $busyText[$script:busy]; $c = $busyChip }
    else { $ui.StateText.Text = $labels[$state]; $c = $chip[$state] }
    $ui.StatePill.Background = Brush $c[0]; $ui.StatePill.BorderBrush = Brush $c[1]; $ui.StateText.Foreground = Brush $c[2]
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
        $dot.Fill = if ($up) { Brush '#47D18B' } else { Brush '#8D8C9C' }
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
        $k = New-Object System.Windows.Controls.TextBlock; $k.Text = $r[0]; $k.Foreground = Brush '#8D8C9C'; $k.Margin = '0,4'
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
    if (-not $m.runnable) { return @('Este runtime no lo soporta todavía', '#F2626B') }
    $sizeMb = $m.bytes / 1MB
    $vram = [double]$l.vram_total_mb
    $ram = [double]$l.ram_total_mb
    # One memory for CPU and GPU (DGX Spark, RTX Spark): the model, its cache and the system
    # share it, so 8 GB stay for the rest.
    if ($l.unified_memory -eq $true) {
        if ($sizeMb + 8192 -le $vram) { return @('Cabe en la memoria unificada', '#47D18B') }
        return @('No cabe en este equipo', '#F2626B')
    }
    if ($vram -gt 0 -and $sizeMb + 1500 -le $vram) { return @('Cabe en la GPU', '#47D18B') }
    if ($vram -gt 0 -and $sizeMb + 1500 -le $vram + 0.5 * $ram) { return @('GPU y CPU, más lento', '#F0B429') }
    if ($vram -le 0 -and $sizeMb -le 0.5 * $ram) { return @('Solo CPU, lento', '#F0B429') }
    return @('No cabe en este equipo', '#F2626B')
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
        if (-not $script:cfg.show_all_models -and -not $m.here -and $fit[1] -eq '#F2626B') { $hidden++; continue }
        $card = New-Object System.Windows.Controls.Border
        $card.Background = Brush '#22232A'
        $card.BorderBrush = Brush $(if ($m.chosen) { '#6FC8F5' } elseif ($m.answering) { '#47D18B' } else { '#2C2D36' })
        $card.BorderThickness = 1; $card.CornerRadius = 8; $card.Padding = '12,8'; $card.Margin = '0,0,0,6'
        $dock = New-Object System.Windows.Controls.DockPanel
        $buttons = New-Object System.Windows.Controls.StackPanel
        $buttons.Orientation = 'Horizontal'; [System.Windows.Controls.DockPanel]::SetDock($buttons, 'Right')
        $info = New-Object System.Windows.Controls.StackPanel
        $title = New-Object System.Windows.Controls.TextBlock
        $title.Text = "$($m.label)  ·  $(Format-Bytes $m.bytes)"; $title.FontWeight = 'SemiBold'
        $sub = New-Object System.Windows.Controls.TextBlock
        $sub.Foreground = Brush '#8D8C9C'; $sub.FontSize = 12
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
    # One voice: a second would hear the first through the speakers and answer it.
    # (archie-talk also refuses a second one; this spares the window it would open.)
    Wsl "pgrep -f '[a]rchie-voice converse'" { param($out, $code)
        if ($code -eq 0) { Add-Log 'La voz ya está abierta.'; return }
        $always = if ($script:cfg.voice_always) { ' --always' } else { ' --named' }
        Start-Process wsl.exe -ArgumentList "-d $($script:cfg.distro) -- archie-talk --local$always"
    }
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

# ---- installing ArchieOS ----------------------------------------------------------------
# Each step is ArchieOS-Installer.ps1 run hidden, as its own process: its lines go to the
# log as they come and its result, one line of JSON, to the callback. What the steps leave
# in <folder>\state is what this section shows, so it holds across closing the window, a
# reboot, or a step that was cut short.
$script:installBusy = ''
$script:installPlan = $null
$script:installManifest = $null
$script:installState = [pscustomobject]@{ state = 'not_installed' }
if (Get-Command Read-ReleaseManifest -ErrorAction SilentlyContinue) {
    try { $script:installManifest = Read-ReleaseManifest } catch { $script:installManifestError = $_.Exception.Message }
}
$installLabels = @{
    not_installed = 'Sin instalar'; checking = 'Comprobando…'; needs_dependencies = 'Faltan requisitos'; downloading = 'Descargando…'
    verifying = 'Verificando…'; installing = 'Instalando…'; needs_reboot = 'Reinicia Windows'; installed = 'ArchieOS instalado'
    starting = 'Iniciando…'; running = 'En ejecución'; stopped = 'Detenido'; error = 'Error'; repairable = 'Necesita reparación'
}
$statusColors = @{ installed = '#47D18B'; warning = '#F0B429'; outdated = '#F0B429'; reboot_required = '#F0B429'; missing = '#F2626B'; unsupported = '#F2626B' }

function Get-DefaultInstallRoot {
    # The fixed NTFS drive with the most room: the system drive is often the fullest.
    $best = [System.IO.DriveInfo]::GetDrives() | Where-Object { $_.DriveType -eq 'Fixed' -and $_.IsReady -and $_.DriveFormat -eq 'NTFS' } |
        Sort-Object AvailableFreeSpace -Descending | Select-Object -First 1
    if ($best) { return (Join-Path $best.RootDirectory.FullName 'ArchieOS') }
    return (Join-Path $env:LOCALAPPDATA 'ArchieOS')
}
function Get-InstallRoot {
    if ($script:cfg.install_root) { return [string]$script:cfg.install_root }
    return $ui.InstRoot.Text.Trim()
}
function Get-InstallDistro {
    $name = $ui.InstDistro.Text.Trim()
    if ($name) { return $name }
    return [string](Get-Target $script:installManifest).distro_name
}
function Start-InstallerJob([string]$verb, [string[]]$more, [scriptblock]$then) {
    $out = [System.IO.Path]::GetTempFileName()
    $result = [System.IO.Path]::GetTempFileName()
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $installer, $verb, '-OutFile', $out, '-ResultFile', $result) + $more
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = 'powershell.exe'
    $psi.Arguments = ConvertTo-ArgumentString $argv
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $script:installBusy = $verb
    Update-InstallView
    $p = [System.Diagnostics.Process]::Start($psi)
    [void]$script:jobs.Add(@{ process = $p; out = $out; label = 'install'; read = 0; data = @{ verb = $verb; result = $result; then = $then }
        then = { param($text, $code, $d)
            $script:installBusy = ''
            $r = Get-LastJsonLine (Read-Output $d.result)
            Remove-Item $d.result -ErrorAction SilentlyContinue
            if (-not $r) { $r = [pscustomobject]@{ ok = $false; error = "El instalador terminó sin respuesta (código $code)."; code = 'unexpected' } }
            try { if ($d.then) { & $d.then $r } } finally { Update-InstallView }
        } })
}
function Show-Message([string]$text, [string]$buttons = 'OK', [string]$icon = 'Information') {
    return [System.Windows.MessageBox]::Show($text, 'ArchieOS', $buttons, $icon)
}
function New-TextBlock([string]$text, [string]$color = '', [int]$size = 0, [string]$weight = '') {
    $t = New-Object System.Windows.Controls.TextBlock
    $t.Text = $text
    if ($color) { $t.Foreground = Brush $color }
    if ($size) { $t.FontSize = $size }
    if ($weight) { $t.FontWeight = $weight }
    return $t
}

function Update-InstallSpace {
    if (-not $script:installManifest) { return }
    $root = $ui.InstRoot.Text.Trim()
    if (-not $root -or -not [System.IO.Path]::IsPathRooted($root)) { $ui.InstSpace.Text = 'Elige una carpeta (una ruta completa, como X:\ArchieOS).'; return }
    try {
        $vol = Get-FolderVolume $root
        $image = Get-ImageArtifact $script:installManifest
        $need = Get-RequiredBytes $root $image (Get-Target $script:installManifest) $script:installState
        $free = if ($vol.ready) { "En $($vol.root) quedan $(Format-Bytes $vol.free) · " } else { "La unidad $($vol.root) no está disponible · " }
        $ui.InstSpace.Text = ("{0}hacen falta {1} (descarga {2} + distribución {3}, estimado del release).`nQuedará: {4}\downloads\{5} · {4}\wsl\{6}\ext4.vhdx · {4}\state · {4}\logs" -f `
            $free, (Format-Bytes $need.total), (Format-Bytes $need.download), (Format-Bytes $need.install), $root, $image.file, (Get-InstallDistro))
    } catch { $ui.InstSpace.Text = "No se puede usar esa carpeta: $($_.Exception.Message)" }
}

function Update-InstallView {
    if (-not $script:installManifest) {
        $ui.InstallTitle.Text = 'Instalador no disponible'
        $ui.InstallSummary.Text = $(if ($script:installManifestError) { $script:installManifestError } else { 'Faltan ArchieOS-Installer.ps1 y archieos-manifest.json junto al lanzador.' })
        foreach ($b in 'InstallStart', 'InstallOpen', 'InstallStop', 'InstallVerify', 'InstallLogs', 'InstGo', 'InstAnalyze', 'InstFixDeps', 'InstBrowse', 'InstTokenSave', 'InstTokenForget', 'InstCancel') { $ui[$b].IsEnabled = $false }
        return
    }
    $root = [string]$script:cfg.install_root
    $st = if ($root) { Read-InstallState $root } else { [pscustomobject]@{ state = 'not_installed' } }
    $script:installState = $st
    $state = [string]$st.state
    $busy = [bool]$script:installBusy
    # A step under way in a process this window did not start (it was closed and reopened).
    $elsewhere = $false
    if (-not $busy -and $st.job_pid) { $elsewhere = [bool](Get-Process -Id $st.job_pid -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -eq 'powershell' }) }
    $active = $state -in @('checking', 'downloading', 'verifying', 'installing', 'starting')
    $interrupted = $active -and -not $busy -and -not $elsewhere
    $installedish = [bool]$st.distro -and (($st.checkpoints -contains 'initialized') -or $st.adopted) -and $state -in @('installed', 'starting', 'running', 'stopped', 'error', 'repairable')

    $ui.InstallTitle.Text = if ($interrupted) { 'Instalación interrumpida' } else { $installLabels[$state] }
    $lines = New-Object System.Collections.ArrayList
    if ($st.distro) { [void]$lines.Add("WSL · $($st.distro)$(if ($st.release) { " · $($st.release)" })") }
    if ($st.location) { [void]$lines.Add($st.location) }
    if ($interrupted) { [void]$lines.Add('Se cortó a medias. Continuar instalación sigue donde quedó (la descarga se reanuda).') }
    elseif ($state -eq 'not_installed') { [void]$lines.Add('Descarga ArchieOS desde su release, lo comprueba y lo instala en WSL 2.') }
    elseif ($state -eq 'running' -and $st.health) { [void]$lines.Add($st.health.detail) }
    elseif ($state -eq 'needs_dependencies') { [void]$lines.Add('Avanzado → Instalar muestra qué falta y cómo resolverlo.') }
    elseif ($state -eq 'needs_reboot') { [void]$lines.Add('WSL se instaló: reinicia Windows y vuelve a abrir el lanzador para seguir.') }
    if ($st.error -and $state -in @('error', 'repairable', 'needs_dependencies')) { [void]$lines.Add($st.error) }
    $ui.InstallSummary.Text = $lines -join "`n"

    $ui.InstallStart.Visibility = if ($installedish) { 'Visible' } else { 'Collapsed' }
    $ui.InstallStart.IsEnabled = -not $busy -and -not $elsewhere -and $state -in @('installed', 'stopped', 'error')
    $ui.InstallStart.Content = if ($state -eq 'running') { 'ArchieOS en ejecución' } else { 'Iniciar ArchieOS' }
    $ui.InstallOpen.Visibility = if ($installedish) { 'Collapsed' } else { 'Visible' }
    $ui.InstallOpen.Content = if ($interrupted -or ($state -eq 'error' -and -not $installedish)) { 'Continuar instalación' } else { 'Instalar ArchieOS' }
    $primary = -not $installedish
    $ui.InstallOpen.Background = Brush $(if ($primary) { '#6FC8F5' } else { '#2A2B32' })
    $ui.InstallOpen.Foreground = Brush $(if ($primary) { '#06151D' } else { '#F2F1EE' })
    $ui.InstallOpen.BorderBrush = Brush $(if ($primary) { '#6FC8F5' } else { '#2C2D36' })
    $ui.InstallStop.Visibility = $ui.InstallStart.Visibility
    $ui.InstallStop.IsEnabled = -not $busy -and $state -in @('running', 'error')
    $ui.InstallVerify.Visibility = $ui.InstallStart.Visibility
    $ui.InstallVerify.IsEnabled = -not $busy -and -not $elsewhere -and [bool]$st.distro
    $cancellable = $script:installBusy -in @('install', 'repair') -or ($elsewhere -and $state -eq 'downloading')
    $ui.InstallCancel.Visibility = if ($cancellable) { 'Visible' } else { 'Collapsed' }
    $ui.InstCancel.IsEnabled = $cancellable
    $ui.InstallLogs.IsEnabled = [bool]$root

    # Avanzado → Instalar.
    if (-not $ui.InstRoot.IsKeyboardFocused -and -not $ui.InstRoot.Text) { $ui.InstRoot.Text = $(if ($root) { $root } else { Get-DefaultInstallRoot }) }
    if (-not $ui.InstDistro.IsKeyboardFocused -and -not $ui.InstDistro.Text) {
        $ui.InstDistro.Text = $(if ($st.distro) { $st.distro } elseif ($script:cfg.install_distro) { $script:cfg.install_distro } else { (Get-Target $script:installManifest).distro_name })
    }
    $locked = $busy -or $elsewhere -or $installedish
    $ui.InstRoot.IsEnabled = -not $locked; $ui.InstBrowse.IsEnabled = -not $locked; $ui.InstDistro.IsEnabled = -not $locked
    $ui.InstAnalyze.IsEnabled = -not $busy
    $ui.InstFixDeps.IsEnabled = -not $busy -and $script:installPlan -and @($script:installPlan | Where-Object { $_.fix_id }).Count -gt 0
    $ui.InstGo.IsEnabled = -not $busy -and -not $elsewhere -and -not $installedish
    $ui.InstGo.Content = $ui.InstallOpen.Content
    $ui.InstModeNote.Text = 'Windows sigue siendo el anfitrión; ArchieOS (Arch Linux) corre en WSL 2 con su propia lógica, permisos y modelos. ' +
        (($script:installManifest.unavailable | Where-Object { $_.id -in @('virtualbox', 'linux-native') } | ForEach-Object { $_.reason }) -join ' ')

    $image = Get-ImageArtifact $script:installManifest
    if ($image.auth) {
        $saved = Test-ProviderToken $image.auth
        $ui.InstAuthNote.Text = ("La imagen está en un repositorio privado de {0}: hace falta un token de una cuenta con acceso de lectura. {1}" -f (Get-AuthHost $image.auth),
            $(if ($saved) { 'Hay un token guardado, cifrado para tu usuario de Windows (DPAPI).' } else { 'No hay ninguno guardado.' }))
        $ui.InstTokenForget.IsEnabled = $saved
    } else { $ui.InstAuthNote.Text = 'La descarga es pública: no hace falta token.'; $ui.InstToken.IsEnabled = $false; $ui.InstTokenSave.IsEnabled = $false; $ui.InstTokenForget.IsEnabled = $false }

    $ui.InstArtifacts.Children.Clear()
    foreach ($a in (Get-Target $script:installManifest).artifacts) {
        $card = New-Object System.Windows.Controls.Border
        $card.Background = Brush '#22232A'; $card.BorderBrush = Brush '#2C2D36'; $card.BorderThickness = 1; $card.CornerRadius = 8; $card.Padding = '12,8'; $card.Margin = '0,0,0,6'
        $info = New-Object System.Windows.Controls.StackPanel
        [void]$info.Children.Add((New-TextBlock "$($a.id) · $($a.file) · $(Format-Bytes $a.size_bytes)" '' 0 'SemiBold'))
        $from = if ($a.source) { ([Uri]$a.source).Host } else { "dentro de $($a.embedded_in); se comprueba con archie-offline --check" }
        [void]$info.Children.Add((New-TextBlock ("{0} · SHA-256 {1}…{2} · {3} · {4}" -f $a.version, $a.sha256.Substring(0, 8), $a.sha256.Substring(56), $from, $(if ($a.required) { 'obligatorio' } else { 'opcional' })) '#8D8C9C' 12))
        $where = ''
        if ($a.source -and $root) {
            $dest = Join-Path $root "downloads\$($a.file)"
            $done = @($st.artifacts | Where-Object { $_.id -eq $a.id -and $_.verified_at })
            if ($done.Count) { $where = 'descargado y comprobado' }
            elseif (Test-Path -LiteralPath $dest) { $where = 'descargado; se comprobará' }
            elseif (Test-Path -LiteralPath "$dest.part") { $where = ('descarga a medias · {0:P0}' -f ((Get-Item -LiteralPath "$dest.part").Length / [double]$a.size_bytes)) }
            else { $where = 'no descargado' }
        } elseif ($a.embedded_in) {
            $where = if ($st.checkpoints -contains 'initialized') { 'comprobado dentro de la instalación' } else { 'llega con la imagen' }
        }
        if ($where) { [void]$info.Children.Add((New-TextBlock $where '#B9B8C4' 12)) }
        $card.Child = $info
        [void]$ui.InstArtifacts.Children.Add($card)
    }
    Update-InstallDeps
    Update-InstallProgress
}
function Update-InstallDeps {
    $ui.InstDeps.Children.Clear()
    if (-not $script:installPlan) { [void]$ui.InstDeps.Children.Add((New-TextBlock 'Analizar revisa este equipo: nada se cambia sin preguntar.' '#8D8C9C' 12)); return }
    foreach ($d in $script:installPlan) {
        $row = New-Object System.Windows.Controls.DockPanel
        $row.Margin = '0,3'
        $dot = New-Object System.Windows.Shapes.Ellipse
        $dot.Width = 9; $dot.Height = 9; $dot.Margin = '0,4,10,0'; $dot.VerticalAlignment = 'Top'
        $dot.Fill = Brush $statusColors[[string]$d.status]
        [System.Windows.Controls.DockPanel]::SetDock($dot, 'Left')
        $text = New-Object System.Windows.Controls.StackPanel
        [void]$text.Children.Add((New-TextBlock "$($d.name) · $($d.detail)"))
        if ($d.fix -and $d.status -ne 'installed') { [void]$text.Children.Add((New-TextBlock $d.fix '#8D8C9C' 12)) }
        [void]$row.Children.Add($dot); [void]$row.Children.Add($text)
        [void]$ui.InstDeps.Children.Add($row)
    }
}
function Update-InstallProgress {
    $root = [string]$script:cfg.install_root
    $file = if ($root) { Join-Path $root 'state\progress.json' } else { '' }
    $show = $file -and (Test-Path -LiteralPath $file) -and $script:installState.state -in @('downloading', 'verifying', 'installing')
    $p = $null
    if ($show) { try { $p = Get-Content -LiteralPath $file -Raw -Encoding UTF8 | ConvertFrom-Json } catch { $show = $false } }
    if ($show -and $p) {
        $text = switch ($p.phase) {
            'downloading' {
                $eta = if ($p.eta_s) { ' · quedan {0}' -f $(if ($p.eta_s -ge 3600) { '{0:N1} h' -f ($p.eta_s / 3600) } elseif ($p.eta_s -ge 60) { '{0:N0} min' -f ($p.eta_s / 60) } else { "$($p.eta_s) s" }) } else { '' }
                'Descargando {0} · {1} de {2} · {3}/s{4}' -f $p.file, (Format-Bytes $p.received), (Format-Bytes $p.total), (Format-Bytes $p.speed_bps), $eta
            }
            'verifying' { 'Verificando el formato de la imagen' }
            default { 'Importando en WSL y comprobando cada modelo (unos minutos)' }
        }
        $value = if ($p.total -gt 0) { [double]$p.received / [double]$p.total } else { 0 }
        foreach ($bar in $ui.InstallBar, $ui.InstBar) { $bar.Visibility = 'Visible'; $bar.IsIndeterminate = $p.phase -ne 'downloading'; $bar.Value = $value }
        foreach ($t in $ui.InstallProgressText, $ui.InstProgressText) { $t.Visibility = 'Visible'; $t.Text = $text }
    } else {
        foreach ($bar in $ui.InstallBar, $ui.InstBar) { $bar.Visibility = 'Collapsed' }
        $ui.InstallProgressText.Visibility = 'Collapsed'
        $ui.InstProgressText.Text = ''
    }
}

function Invoke-InstallAnalyze {
    $root = $ui.InstRoot.Text.Trim()
    $more = @('-Distro', (Get-InstallDistro))
    if ($root -and [System.IO.Path]::IsPathRooted($root)) { $more += @('-Root', $root) }
    Add-Log 'Instalar: analizando este equipo'
    Start-InstallerJob 'plan' $more { param($r)
        if (-not $r.ok) { Add-Log "Instalar: $($r.error)"; return }
        $script:installPlan = @($r.plan)
        $p = $r.probe
        $ui.InstSystem.Text = if ($p.platform -ne 'windows') { "$($p.os) · $($p.arch)" } else {
            ("{0} · compilación {1} · {2} · {3} · {4}" -f $p.os, $p.build, $(if ($p.arch -eq 'AMD64') { 'x86_64' } else { $p.arch }),
                $(if ($p.wsl.version) { "WSL $($p.wsl.version)" } else { 'sin WSL' }),
                $(if ($p.admin) { 'como administrador' } else { 'sin permisos de administrador (se piden solo para instalar requisitos)' })) }
        $blocking = @($script:installPlan | Where-Object { $_.blocking })
        Add-Log $(if ($blocking.Count) { "Instalar: faltan $($blocking.Count) requisitos" } else { 'Instalar: el equipo cumple los requisitos' })
        Update-InstallSpace
    }
}
function Invoke-InstallFixDeps {
    $fixes = @($script:installPlan | Where-Object { $_.fix_id } | Select-Object -ExpandProperty fix_id -Unique)
    if (-not $fixes.Count) { return }
    $what = ($script:installPlan | Where-Object { $_.fix_id } | ForEach-Object { "• $($_.name): $($_.fix)" } | Select-Object -Unique) -join "`n"
    $reboot = @($script:installPlan | Where-Object { $_.fix_id -and $_.reboot }).Count -gt 0
    $answer = Show-Message ("Se va a ejecutar, con permiso de administrador (Windows lo pedirá):`n`n{0}`n`n{1}¿Seguir?" -f $what,
        $(if ($reboot) { "Después hay que reiniciar Windows; el lanzador sigue donde quedó.`n`n" } else { '' })) 'YesNo' 'Question'
    if ($answer -ne 'Yes') { return }
    $id = if ($fixes -contains 'wsl') { 'wsl' } else { $fixes[0] }
    $root = $ui.InstRoot.Text.Trim()
    if ($root) { $script:cfg.install_root = $root; Save-Settings }
    Start-InstallerJob 'fix-dependency' (@('-Id', $id) + $(if ($root) { @('-Root', $root) } else { @() })) { param($r)
        if (-not $r.ok) { Show-Message $r.error 'OK' 'Warning' | Out-Null; return }
        if ($r.reboot) { Show-Message 'WSL está instalado. Reinicia Windows y vuelve a abrir el lanzador para seguir con la instalación.' | Out-Null }
        Invoke-InstallAnalyze
    }
}
function Invoke-InstallGo {
    $root = $ui.InstRoot.Text.Trim()
    $distro = Get-InstallDistro
    if (-not $root -or -not [System.IO.Path]::IsPathRooted($root)) { Show-Message 'Elige una carpeta con su ruta completa, como X:\ArchieOS.' | Out-Null; return }
    if ($distro -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') { Show-Message 'El nombre de la distribución lleva solo letras, números, punto, guion y guion bajo.' | Out-Null; return }
    $image = Get-ImageArtifact $script:installManifest
    $cached = Test-Path -LiteralPath (Join-Path $root "downloads\$($image.file)")
    if ($image.auth -and -not $cached -and -not (Test-ProviderToken $image.auth)) {
        Show-Message ("La imagen está en un repositorio privado de {0}. Guarda primero un token en Avanzado → Instalar → Acceso a la descarga." -f (Get-AuthHost $image.auth)) | Out-Null
        return
    }
    $need = Get-RequiredBytes $root $image (Get-Target $script:installManifest) $script:installState
    $answer = Show-Message ("Instalar ArchieOS {0}:`n`n• Descargar {1} ({2}) de {3}, con su SHA-256 comprobado, a {4}\downloads`n• Importarla en WSL como {5}, con su disco en {4}\wsl\{5}`n• Preparar sus claves y comprobar cada modelo (IRIS incluido)`n`nHacen falta unos {6} libres. No se toca ninguna otra distribución. ¿Seguir?" -f `
        $script:installManifest.release, $image.file, (Format-Bytes $need.download), ([Uri]$image.source).Host, $root, $distro, (Format-Bytes $need.total)) 'YesNo' 'Question'
    if ($answer -ne 'Yes') { return }
    $script:cfg.install_root = $root; $script:cfg.install_distro = $distro; Save-Settings
    Add-Log "Instalar: $root ($distro)"
    Start-InstallerJob 'install' @('-Root', $root, '-Distro', $distro) { param($r)
        if ($r.ok -and $r.state -eq 'installed') {
            $tray.ShowBalloonTip(4000, 'ArchieOS', 'ArchieOS está instalado. Iniciar ArchieOS lo pone en marcha.', 'Info')
            Show-Message ("ArchieOS instalado.`n`nUbicación: {0}`nVersión: {1}`nDistribución: {2}" -f $r.location, $r.release, $r.distro) | Out-Null
        } elseif ($r.ok -and $r.state -in @('needs_dependencies', 'needs_reboot')) {
            $script:installPlan = @($r.plan)
            $ui.AdvancedToggle.IsChecked = $true; $ui.InstallTab.IsSelected = $true
        } elseif ($r.code -eq 'distro_exists') { Invoke-InstallAdoptOffer $r.error }
        elseif ($r.code -eq 'repairable') { Invoke-InstallRepair }
        elseif ($r.code -ne 'cancelled') { Show-Message "La instalación no terminó.`n`n$($r.error)`n`nRegistro: $(Join-Path $root 'logs\installer.log')" 'OK' 'Warning' | Out-Null }
    }
}
function Invoke-InstallAdoptOffer([string]$why) {
    $distro = Get-InstallDistro
    $answer = Show-Message "$why`n`n¿Reutilizar esa distribución? Antes se comprueba con su propio archie-offline --check; no se importa ni se borra nada.`n`n(No: cambia el nombre de la distribución en Avanzado → Instalar para instalar aparte.)" 'YesNo' 'Question'
    if ($answer -ne 'Yes') { return }
    Start-InstallerJob 'adopt' @('-Root', (Get-InstallRoot), '-Distro', $distro) { param($r)
        if (-not $r.ok) { Show-Message "No se puede reutilizar: $($r.error)" 'OK' 'Warning' | Out-Null }
    }
}
function Invoke-InstallRepair {
    $st = $script:installState
    $more = @('-Root', (Get-InstallRoot))
    $reg = if ($st.distro) { Get-WslRegistration $st.distro } else { $null }
    if ($reg -and -not $reg.disk_present) {
        $answer = Show-Message ("{0} está registrada en WSL, pero su disco ya no existe:`n{1}`n`nReparar quita ese registro (wsl --unregister {0}; no queda nada que perder) y vuelve a importar la imagen descargada. ¿Seguir?" -f $st.distro, $reg.vhdx) 'YesNo' 'Warning'
        if ($answer -ne 'Yes') { return }
        $more += '-AllowUnregisterBroken'
    } else {
        $answer = Show-Message 'Reparar vuelve a preparar la distribución (claves y carpeta de estado) y comprueba cada modelo; si falta, la reimporta desde la imagen descargada. No borra datos. ¿Seguir?' 'YesNo' 'Question'
        if ($answer -ne 'Yes') { return }
    }
    Start-InstallerJob 'repair' $more { param($r)
        Show-Message $(if ($r.ok) { 'Reparado: la instalación pasa su comprobación.' } else { "No se pudo reparar.`n`n$($r.error)" }) | Out-Null
    }
}
function Invoke-InstallCancel {
    $root = Get-InstallRoot
    if (-not $root) { return }
    # The download stops at its next block and keeps what it has; an import under way ends first.
    New-Item -ItemType File -Force (Join-Path $root 'state\cancel') | Out-Null
    Add-Log 'Instalar: cancelando (lo descargado se conserva)'
}

$ui.InstallOpen.Add_Click({
    $ui.AdvancedToggle.IsChecked = $true; $ui.InstallTab.IsSelected = $true
    if ($ui.InstallOpen.Content -eq 'Continuar instalación') { Invoke-InstallGo } elseif (-not $script:installPlan) { Invoke-InstallAnalyze }
})
$ui.InstGo.Add_Click({ Invoke-InstallGo })
$ui.InstAnalyze.Add_Click({ Invoke-InstallAnalyze })
$ui.InstFixDeps.Add_Click({ Invoke-InstallFixDeps })
$ui.InstallCancel.Add_Click({ Invoke-InstallCancel })
$ui.InstCancel.Add_Click({ Invoke-InstallCancel })
$ui.InstBrowse.Add_Click({
    $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
    $dialog.Description = 'Carpeta para ArchieOS: la descarga, el disco de WSL, su estado y sus registros'
    $dialog.ShowNewFolderButton = $true
    if ($ui.InstRoot.Text -and (Test-Path -LiteralPath $ui.InstRoot.Text)) { $dialog.SelectedPath = $ui.InstRoot.Text }
    if ($dialog.ShowDialog() -eq 'OK') { $ui.InstRoot.Text = $dialog.SelectedPath; Update-InstallSpace }
})
$ui.InstRoot.Add_LostFocus({ Update-InstallSpace })
$ui.InstDistro.Add_LostFocus({ Update-InstallSpace })
$ui.InstTokenSave.Add_Click({
    $image = Get-ImageArtifact $script:installManifest
    if ($ui.InstToken.SecurePassword.Length -eq 0) { return }
    Save-ProviderToken $image.auth $ui.InstToken.SecurePassword
    $ui.InstToken.Clear()
    Add-Log "Instalar: token de $(Get-AuthHost $image.auth) guardado (cifrado con DPAPI)"
    Update-InstallView
})
$ui.InstTokenForget.Add_Click({ Remove-ProviderToken (Get-ImageArtifact $script:installManifest).auth; Add-Log 'Instalar: token olvidado'; Update-InstallView })
$ui.InstallStart.Add_Click({
    Add-Log 'ArchieOS: iniciando'
    Start-InstallerJob 'start' @('-Root', (Get-InstallRoot)) { param($r)
        if ($r.ok) { Add-Log $(if ($r.already) { 'ArchieOS ya estaba en ejecución.' } else { "ArchieOS en ejecución: $($r.health.detail)" }) }
        elseif ($r.code -eq 'repairable') { Invoke-InstallRepair }
        else { Show-Message "ArchieOS no arrancó.`n`n$($r.error)" 'OK' 'Warning' | Out-Null }
    }
})
$ui.InstallStop.Add_Click({
    Start-InstallerJob 'stop' @('-Root', (Get-InstallRoot)) { param($r) if (-not $r.ok) { Show-Message $r.error 'OK' 'Warning' | Out-Null } }
})
$ui.InstallVerify.Add_Click({
    $deep = Show-Message "Verificar comprueba el SHA-256 de cada modelo dentro de la instalación.`n`n¿Cargar también IRIS en la CPU para comprobar que sus pesos se leen? (hasta 10 minutos)" 'YesNoCancel' 'Question'
    if ($deep -eq 'Cancel') { return }
    $more = @('-Root', (Get-InstallRoot)) + $(if ($deep -eq 'Yes') { @('-Deep') } else { @() })
    Start-InstallerJob 'verify' $more { param($r)
        if ($r.ok) {
            $iris = if ($r.iris) { "`nIRIS: $($r.iris.iris)$(if ($r.iris.seconds) { " (respondió en $($r.iris.seconds) s)" } elseif ($r.iris.detail) { " · $($r.iris.detail)" })" } else { '' }
            Show-Message ("Instalación correcta: {0} modelos comprobados (código {1}).{2}" -f $r.integrity.files, $r.integrity.source_sha, $iris) | Out-Null
        } else {
            $answer = Show-Message "La verificación falló.`n`n$($r.error)`n`n¿Reparar?" 'YesNo' 'Warning'
            if ($answer -eq 'Yes') { Invoke-InstallRepair }
        }
    }
})
$ui.InstallLogs.Add_Click({
    $logs = Join-Path (Get-InstallRoot) 'logs'
    New-Item -ItemType Directory -Force $logs | Out-Null
    Start-Process explorer.exe $logs
    # ArchieOS's own logs, inside the distribution, while it runs.
    $st = $script:installState
    if ($st.distro -and (Test-DistroRunning $st.distro)) { Start-Process explorer.exe ("\\wsl.localhost\{0}\home\archie\.local\state\archie-offline" -f $st.distro) }
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
if ($Page -eq 'instalar') {
    $ui.AdvancedToggle.IsChecked = $true; $script:cfg.advanced = $true
    $ui.AdvancedTabs.Visibility = 'Visible'; $ui.PreviewCard.Visibility = 'Collapsed'
    $ui.InstallTab.IsSelected = $true
}

# Every ten seconds, and faster while a model downloads.
$poll = New-Object System.Windows.Threading.DispatcherTimer
$poll.Interval = [TimeSpan]::FromSeconds(10)
$poll.Add_Tick({ if (-not $script:busy) { try { Refresh-All } catch { Add-Log "Error: $($_.Exception.Message)" } } })
$poll.Start()
# Instalar ArchieOS follows its state file; a running ArchieOS whose distribution stopped
# (wsl --shutdown, a crash) is asked again, so the card does not say En ejecución for nothing.
$poll.Add_Tick({
    try {
        Update-InstallView
        $st = $script:installState
        if (-not $script:installBusy -and $st.state -eq 'running' -and $st.distro -and -not (Test-DistroRunning $st.distro)) {
            Start-InstallerJob 'health' @('-Root', (Get-InstallRoot)) { param($r) }
        }
    } catch { Add-Log "Error: $($_.Exception.Message)" }
})

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
$window.Add_ContentRendered({ try { Update-InstallView; Update-InstallSpace } catch { Add-Log "Error: $($_.Exception.Message)" } })
[void]$window.ShowDialog()

#define AppName "KVM Switch"
#define AppPublisher "mpgxc"
#ifndef AppVersion
#define AppVersion GetEnv("KVM_SWITCH_VERSION")
#endif
#define ProjectRoot AddBackslash(SourcePath) + ".."

[Setup]
AppId={{CA5CB73D-CDE9-4B73-A398-86236D7DF2BF}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={localappdata}\Programs\KVM Switch
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
OutputDir={#ProjectRoot}\target\release
OutputBaseFilename=KVM-Switch-Setup-{#AppVersion}
UninstallDisplayIcon={app}\kvm-switch.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "autostart"; Description: "Iniciar com o Windows e manter o ícone na área de notificação"; Flags: checkedonce
Name: "desktopicon"; Description: "Criar atalho na área de trabalho"; Flags: unchecked

[Files]
Source: "{#ProjectRoot}\target\windows-installer-build\release\kvm-switch.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\KVM Switch"; Filename: "{app}\kvm-switch.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\KVM Switch"; Filename: "{app}\kvm-switch.exe"; WorkingDir: "{app}"; Tasks: desktopicon
Name: "{autostartup}\KVM Switch"; Filename: "{app}\kvm-switch.exe"; Parameters: "start"; WorkingDir: "{app}"; Tasks: autostart

[Run]
Filename: "{app}\kvm-switch.exe"; Description: "Abrir o KVM Switch"; WorkingDir: "{app}"; Flags: postinstall nowait skipifsilent

#define MyAppName "Deskbridge"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Deskbridge"
#define MyAppExeName "deskbridge.exe"

[Setup]
AppId={{C2C7F8F1-9A6A-4E3E-9F8E-7B1D2F4A3A10}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\Deskbridge
DefaultGroupName=Deskbridge
DisableProgramGroupPage=yes
OutputDir=..\..\dist\windows
OutputBaseFilename=Deskbridge-Setup-{#MyAppVersion}
SetupIconFile=..\..\assets\deskbridge.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
UninstallDisplayIcon={app}\{#MyAppExeName}

[Files]
Source: "..\..\target\release\deskbridge.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\deskbridge.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Deskbridge"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\deskbridge.ico"
Name: "{group}\Deskbridge CLI"; Filename: "{cmd}"; Parameters: "/K ""{app}\{#MyAppExeName} --help"""; IconFilename: "{app}\deskbridge.ico"
Name: "{group}\Uninstall Deskbridge"; Filename: "{uninstallexe}"

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch Deskbridge"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{userprofile}\.deskbridge"

; Oxplorer installer (Inno Setup 6). Built by installer/build.ps1:
;   ISCC.exe /DAppVersion=0.2.0 /DOutputName=oxplorer-v0.2.0-setup-x64 installer\oxplorer.iss
; Installs for the current user by default (no admin rights needed); the
; first page offers installing for all users instead.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef OutputName
  #define OutputName "oxplorer-setup-x64"
#endif

[Setup]
AppId={{6F0C9E57-3B8A-4E4B-9C2D-0B7D1A54E2F1}
AppName=Oxplorer
AppVersion={#AppVersion}
AppVerName=Oxplorer {#AppVersion}
AppPublisher=Flory-1
AppPublisherURL=https://github.com/Flory-1/oxplorer
AppSupportURL=https://github.com/Flory-1/oxplorer/issues
AppUpdatesURL=https://github.com/Flory-1/oxplorer/releases
DefaultDirName={autopf}\Oxplorer
DefaultGroupName=Oxplorer
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
LicenseFile=..\LICENSE
SetupIconFile=..\crates\ox-app\assets\oxplorer.ico
UninstallDisplayIcon={app}\oxplorer.exe
OutputDir=..\dist
OutputBaseFilename={#OutputName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[CustomMessages]
en.DefaultFolders=Open folders in Oxplorer instead of Explorer (can be changed later in Settings)
de.DefaultFolders=Ordner in Oxplorer statt im Explorer öffnen (später in den Einstellungen änderbar)
en.Integration=Windows integration:
de.Integration=Windows-Integration:

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "defaultfolders"; Description: "{cm:DefaultFolders}"; GroupDescription: "{cm:Integration}"; Flags: unchecked

[Files]
Source: "..\target\release\oxplorer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Oxplorer"; Filename: "{app}\oxplorer.exe"
Name: "{autodesktop}\Oxplorer"; Filename: "{app}\oxplorer.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\oxplorer.exe"; Parameters: "--register-default"; Tasks: defaultfolders; Flags: runhidden waituntilterminated
Filename: "{app}\oxplorer.exe"; Description: "{cm:LaunchProgram,Oxplorer}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Give folders back to Explorer if Oxplorer was opening them.
Filename: "{app}\oxplorer.exe"; Parameters: "--unregister-default"; Flags: runhidden waituntilterminated; RunOnceId: "UnregisterDefault"

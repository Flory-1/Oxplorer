; Oxplorer installer (Inno Setup 6). Built by installer/build.ps1:
;   ISCC.exe /DAppVersion=0.2.0 /DOutputName=oxplorer-v0.2.0-setup-x64 installer\oxplorer.iss
; Installs for the current user by default (no admin rights needed); the
; first page offers installing for all users instead. Only then (admin
; rights) the optional, unchecked "Fast index service" task is offered: it
; installs oxplorer-indexer.exe to Program Files and registers it as an
; on-demand LocalSystem service for the user logged on to this session
; (see ox_core::index::helper for the security model).

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
en.SearchGroup=Search:
de.SearchGroup=Suche:
en.IndexService=Fast index service (needs admin rights): reads NTFS drives for the search index without running Oxplorer as administrator. Shows you the names of all files on the drives. Turn it on in Settings → Search index.
de.IndexService=Schneller Indexdienst (braucht Administratorrechte): liest NTFS-Laufwerke für den Suchindex, ohne Oxplorer als Administrator zu starten. Zeigt Ihnen die Namen aller Dateien auf den Laufwerken. Einschalten unter Einstellungen → Suchindex.
en.IndexServiceNoUser=The index service wasn't set up: no user is logged on to this session.
de.IndexServiceNoUser=Der Indexdienst wurde nicht eingerichtet: In dieser Sitzung ist kein Benutzer angemeldet.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "defaultfolders"; Description: "{cm:DefaultFolders}"; GroupDescription: "{cm:Integration}"; Flags: unchecked
; Only when installing for all users: a LocalSystem service must not run
; from a folder the user can write to.
Name: "indexservice"; Description: "{cm:IndexService}"; GroupDescription: "{cm:SearchGroup}"; Flags: unchecked; Check: IsAdminInstallMode

[Files]
Source: "..\target\release\oxplorer.exe"; DestDir: "{app}"; Flags: ignoreversion
; Always in Program Files (writable only by administrators), even when {app} was changed.
Source: "..\target\release\oxplorer-indexer.exe"; DestDir: "{commonpf}\Oxplorer Index Service"; Flags: ignoreversion; Tasks: indexservice
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; The AppUserModelID is the one the program gives itself (ox_core::toast::APP_ID):
; Windows shows Oxplorer's notifications under it and the taskbar groups
; its windows with the shortcut.
Name: "{autoprograms}\Oxplorer"; Filename: "{app}\oxplorer.exe"; AppUserModelID: "Flory-1.Oxplorer"
Name: "{autodesktop}\Oxplorer"; Filename: "{app}\oxplorer.exe"; AppUserModelID: "Flory-1.Oxplorer"; Tasks: desktopicon

[Run]
Filename: "{app}\oxplorer.exe"; Parameters: "--register-default"; Tasks: defaultfolders; Flags: runhidden waituntilterminated
Filename: "{app}\oxplorer.exe"; Description: "{cm:LaunchProgram,Oxplorer}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Give folders back to Explorer if Oxplorer was opening them.
Filename: "{app}\oxplorer.exe"; Parameters: "--unregister-default"; Flags: runhidden waituntilterminated; RunOnceId: "UnregisterDefault"
; Forget the registration for notifications (HKCU\Software\Classes\AppUserModelId).
Filename: "{app}\oxplorer.exe"; Parameters: "--unregister-notifications"; Flags: runhidden waituntilterminated; RunOnceId: "UnregisterNotifications"

[Code]
const
  IndexService = 'OxplorerIndex';
  IndexServiceKey = 'SYSTEM\CurrentControlSet\Services\OxplorerIndex\Parameters';

function IndexServiceExe: String;
begin
  Result := ExpandConstant('{commonpf}\Oxplorer Index Service\oxplorer-indexer.exe');
end;

procedure RunSc(const Params: String);
var
  Code: Integer;
begin
  if Exec(ExpandConstant('{sys}\sc.exe'), Params, '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Log('sc.exe ' + Params + ' -> ' + IntToStr(Code))
  else
    Log('sc.exe ' + Params + ' could not run');
end;

function IndexServiceExists: Boolean;
var
  Code: Integer;
begin
  Result := Exec(ExpandConstant('{sys}\sc.exe'), 'query ' + IndexService, '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

procedure StopIndexService;
begin
  if IndexServiceExists then
  begin
    RunSc('stop ' + IndexService);
    Sleep(1500);
  end;
end;

{ Removes the service (Windows deletes its Parameters key with it) and its exe. }
procedure RemoveIndexService;
begin
  if IndexServiceExists then
  begin
    StopIndexService;
    RunSc('delete ' + IndexService);
  end;
  if FileExists(IndexServiceExe) then
  begin
    DeleteFile(IndexServiceExe);
    RemoveDir(ExtractFileDir(IndexServiceExe));
  end;
end;

{ A user account SID: S-1-5-21-... (local, domain, Microsoft account) or S-1-12-1-... (Entra ID). }
function IsUserSid(const Sid: String): Boolean;
var
  I: Integer;
begin
  Result := ((Pos('S-1-5-21-', Sid) = 1) or (Pos('S-1-12-1-', Sid) = 1)) and (Length(Sid) < 100);
  if Result then
    for I := 1 to Length(Sid) do
      if not (((Sid[I] >= '0') and (Sid[I] <= '9')) or (Sid[I] = '-') or (Sid[I] = 'S')) then
        Result := False;
end;

{ The user logged on to this session (the person at the screen, also when
  setup was elevated with another account), asked from the indexer. }
function SessionUserSid: String;
var
  F: String;
  S: AnsiString;
  Code: Integer;
begin
  Result := '';
  F := ExpandConstant('{tmp}\oxplorer-session-user.txt');
  if Exec(IndexServiceExe, '--session-user-sid "' + F + '"', '', SW_HIDE, ewWaitUntilTerminated, Code)
     and (Code = 0) and LoadStringFromFile(F, S) then
    Result := Trim(String(S));
  if not IsUserSid(Result) then
    Result := '';
end;

procedure InstallIndexService;
var
  Sid, Bin: String;
begin
  Sid := SessionUserSid;
  if Sid = '' then
  begin
    SuppressibleMsgBox(ExpandConstant('{cm:IndexServiceNoUser}'), mbError, MB_OK, IDOK);
    exit;
  end;
  { Quoted: the path has spaces. }
  Bin := '"\"' + IndexServiceExe + '\""';
  if IndexServiceExists then
    RunSc('config ' + IndexService + ' binPath= ' + Bin + ' type= own start= demand obj= LocalSystem')
  else
    RunSc('create ' + IndexService + ' binPath= ' + Bin + ' type= own start= demand obj= LocalSystem DisplayName= "Oxplorer index service"');
  RunSc('description ' + IndexService + ' "Reads the NTFS master file tables and change journals for the search index of Oxplorer, for one user. Installed by the optional Fast index service task."');
  { Only "bypass traverse checking"; all other privileges are removed. }
  RunSc('privs ' + IndexService + ' SeChangeNotifyPrivilege');
  { The defaults, plus: the allowed user may start it and see its status. }
  RunSc('sdset ' + IndexService + ' D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)(A;;CCLCRPLORC;;;' + Sid + ')');
  { The only user the service's pipe lets in (besides SYSTEM and Administrators). }
  if not RegWriteStringValue(HKLM, IndexServiceKey, 'AllowedUser', Sid) then
    Log('Could not record the allowed user');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if not IsAdminInstallMode then
    exit;
  { The running service would keep its exe from being replaced. }
  if CurStep = ssInstall then
    StopIndexService;
  if CurStep = ssPostInstall then
  begin
    if WizardIsTaskSelected('indexservice') then
      InstallIndexService
    else
      RemoveIndexService;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usUninstall) and IsAdminInstallMode then
    RemoveIndexService;
end;

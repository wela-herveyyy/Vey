; Vey installer (Inno Setup 6). Per-user, no admin rights needed.
; Built by the release workflow:  ISCC.exe /DMyAppVersion=1.2.3 installer\vey.iss
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif

[Setup]
AppId={{A3F5D0C2-6B1E-4F7A-9C2D-5E8B7A1D3F40}
AppName=Vey
AppVersion={#MyAppVersion}
AppVerName=Vey {#MyAppVersion}
AppPublisher=Vey
AppPublisherURL=https://github.com/wela-herveyyy/Vey
AppSupportURL=https://github.com/wela-herveyyy/Vey/issues
DefaultDirName={localappdata}\Programs\Vey
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=Output
OutputBaseFilename=VeySetup-{#MyAppVersion}
SetupIconFile=..\assets\vey.ico
UninstallDisplayIcon={app}\vey.exe
UninstallDisplayName=Vey
LicenseFile=..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
VersionInfoProductName=Vey

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startup"; Description: "Start Vey when &Windows starts"; GroupDescription: "Options:"
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "..\target\release\vey.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; same file the app keeps up to date itself, so Windows search finds "Vey"
Name: "{autoprograms}\Vey"; Filename: "{app}\vey.exe"
Name: "{autodesktop}\Vey"; Filename: "{app}\vey.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Vey"; ValueData: """{app}\vey.exe"""; Flags: uninsdeletevalue; Tasks: startup

[Run]
Filename: "{app}\vey.exe"; Description: "Start Vey now"; Flags: nowait postinstall skipifsilent

[Code]
// Vey has no window to close, so stop it before replacing or removing the file.
procedure StopVey();
var
  rc: Integer;
begin
  Exec('taskkill.exe', '/F /IM vey.exe', '', SW_HIDE, ewWaitUntilTerminated, rc);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopVey();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    StopVey();
    // also remove the start-with-Windows entry if Vey's own tray menu created it
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Vey');
  end;
end;

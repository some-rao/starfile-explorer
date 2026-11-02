; StarFile Explorer — kullanici başına kurulum (yonetici izni istemez).
;
; Derleme (Inno Setup 6):
;   ISCC.exe /DAppVersion=0.8.0 packaging\windows\starfile.iss
;
; Cikti: dist\StarFile-Setup-<surum>.exe
;
; Kaldirici, dosyalari silmeden once `starfile.exe --uninstall-cleanup`
; calistirir: otomatik baslatma kaydi ve varsayilan dosya gezgini anahtarlari
; geri alınır. kullanici ayarlari (%APPDATA%\StarFile) bilerek korunur.

#define AppName "StarFile Explorer"
#define AppExe "starfile.exe"
#define AppPublisher "StarFile Explorer"
#define AppUserModelID "io.github.some_rao.StarFile"

#ifndef AppVersion
  #define AppVersion "0.8.0"
#endif

; Yerel test için: ISCC /DSourceExe=..\..\target\debug\starfile.exe
#ifndef SourceExe
  #define SourceExe "..\..\target\release\starfile.exe"
#endif

[Setup]
AppId={{3C6F1A9D-72B4-4E58-9D31-A0C5E8B74F26}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\..\dist
OutputBaseFilename=StarFile-Setup-{#AppVersion}
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no
AllowNoIcons=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; Kısayoldaki AUMID, surecteki AUMID ile aynı olmalı; yoksa görev çubuğu
; Uygulamayı ayrı bir simge olarak gruplar ve sabitleme çalışmaz.
Name: "{userprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; AppUserModelID: "{#AppUserModelID}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Tasks: desktopicon; AppUserModelID: "{#AppUserModelID}"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#AppExe}"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#AppExe}"; ValueType: string; ValueName: "Path"; ValueData: "{app}"; Flags: uninsdeletekey

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#StringChange(AppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Dosyalar silinmeden once çalışır (bkz. src/app.rs `uninstall_cleanup`)
Filename: "{app}\{#AppExe}"; Parameters: "--uninstall-cleanup"; Flags: runhidden; RunOnceId: "RegistryCleanup"

[UninstallDelete]
Type: filesandordirs; Name: "{app}"

[Code]
// Guncelleme/kurulum sirasinda calisan surec exe dosyasini kilitlemesin.
// Uygulama arka planda bekliyorsa WM_CLOSE ile kapanmaz; bu yuzden sonlandirilir.
procedure StopRunningApp();
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{cmd}'), '/c taskkill /F /IM {#AppExe} >nul 2>&1', '',
       SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Sleep(250);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningApp();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopRunningApp();
  Result := True;
end;

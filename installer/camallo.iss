; Inno Setup script for Camallo. Built in CI by .github/workflows/installer.yml:
;   ISCC /DMyAppVersion=0.6.0 /DSourceDir=dist installer\camallo.iss
; Per-user install: no administrator rights, files under %LOCALAPPDATA%\Programs\Camallo.

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "dist"
#endif

#define MyAppName "Camallo"
#define MyAppExe "camallo.exe"

[Setup]
; Fixed forever: it is how Windows (and the next installer) recognises this app for upgrades and uninstall.
AppId={{4DDB6222-205E-4E02-BB38-79A24C5A9EBC}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher=echidistanti
AppPublisherURL=https://github.com/echidistanti/camallo
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExe}
UninstallDisplayName={#MyAppName}
OutputDir=output
OutputBaseFilename=camallo-{#MyAppVersion}-windows-x64-setup
Compression=lzma2/ultra
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"

[CustomMessages]
english.RemoveDataPrompt=Also remove Camallo settings, history, yt-dlp and ffmpeg?%n%nYour downloaded videos are never touched.
italian.RemoveDataPrompt=Rimuovere anche impostazioni, cronologia, yt-dlp e ffmpeg di Camallo?%n%nI video scaricati non vengono mai toccati.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; The built-in browser (WebView2) keeps its profile next to the exe, created at run time.
Type: filesandordirs; Name: "{app}\{#MyAppExe}.WebView2"

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep <> usPostUninstall then Exit;
  DataDir := ExpandConstant('{localappdata}\Camallo');
  if not DirExists(DataDir) then Exit;
  { A silent uninstall keeps user data; an interactive one asks. }
  if UninstallSilent then Exit;
  if MsgBox(CustomMessage('RemoveDataPrompt'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    DelTree(DataDir, True, True, True);
end;

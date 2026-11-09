; Inno Setup script for IPTV. Built by build-installer.ps1, which defines
; AppVersion, StageDir and OutputDir on the command line.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
; Keep AppId stable forever: it is how upgrades and uninstall find the app.
AppId={{3D5BE745-2AB7-4B76-9A73-3B792CD7D9CA}
AppName=IPTV
AppVersion={#AppVersion}
AppVerName=IPTV {#AppVersion}
AppPublisher=IPTV
; Per-user install: no administrator prompt.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\IPTV
DefaultGroupName=IPTV
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename=IPTV-Setup-{#AppVersion}
SetupIconFile=iptv.ico
UninstallDisplayIcon={app}\iptv.exe
UninstallDisplayName=IPTV
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\IPTV"; Filename: "{app}\iptv.exe"
Name: "{autodesktop}\IPTV"; Filename: "{app}\iptv.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\iptv.exe"; Description: "{cm:LaunchProgram,IPTV}"; Flags: nowait postinstall skipifsilent
; Automatic updates: IPTV runs the new installer with /VERYSILENT /RELAUNCH
; and closes; the installer then starts the updated IPTV again.
Filename: "{app}\iptv.exe"; Flags: nowait; Check: ShouldRelaunch

[Code]
function ShouldRelaunch: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/RELAUNCH') = 0 then
      Result := True;
end;

[UninstallDelete]
; Downloaded playlist cache. Favorites/last channel (in %APPDATA%\iptv) are
; kept so a reinstall remembers them.
Type: filesandordirs; Name: "{localappdata}\iptv"

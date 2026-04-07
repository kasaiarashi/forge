; Forge VCS Server Installer for Windows
; Installs forge-server, forge-web, UI assets, and config templates

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif

#ifndef ArtifactDir
  #define ArtifactDir "..\..\artifacts"
#endif

#ifndef ConfigDir
  #define ConfigDir "..\config"
#endif

[Setup]
AppName=Forge VCS Server
AppVersion={#AppVersion}
AppPublisher=Krishna Teja
AppPublisherURL=https://github.com/nicholasgasior/forge
DefaultDirName={autopf}\ForgeServer
DefaultGroupName=Forge VCS Server
OutputBaseFilename=ForgeServer-Windows-x64-Setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesEnvironment=yes
PrivilegesRequired=admin
WizardStyle=modern
SetupIconFile=compiler:SetupClassicIcon.ico

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
; Server binaries
Source: "{#ArtifactDir}\forge-server-windows\forge-server.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ArtifactDir}\forge-server-windows\forge-web.exe"; DestDir: "{app}"; Flags: ignoreversion

; Web UI assets
Source: "{#ArtifactDir}\forge-web-ui-dist\*"; DestDir: "{app}\ui"; Flags: ignoreversion recursesubdirs createallsubdirs

; Config templates (don't overwrite existing configs)
Source: "{#ConfigDir}\forge-server.toml"; DestDir: "{app}"; Flags: onlyifdoesntexist
Source: "{#ConfigDir}\forge-web.toml"; DestDir: "{app}"; Flags: onlyifdoesntexist

[Registry]
; Add {app} to system PATH
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; \
    ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
    Check: NeedsAddPath(ExpandConstant('{app}'))

[Icons]
Name: "{group}\Start Forge Server"; Filename: "{app}\forge-server.exe"; Parameters: "--config ""{app}\forge-server.toml"""; WorkingDir: "{app}"
Name: "{group}\Start Forge Web UI"; Filename: "{app}\forge-web.exe"; Parameters: "--config ""{app}\forge-web.toml"""; WorkingDir: "{app}"
Name: "{group}\Edit Server Config"; Filename: "notepad.exe"; Parameters: """{app}\forge-server.toml"""
Name: "{group}\Edit Web Config"; Filename: "notepad.exe"; Parameters: """{app}\forge-web.toml"""
Name: "{group}\Uninstall Forge Server"; Filename: "{uninstallexe}"

[Code]
function NeedsAddPath(Param: string): Boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE,
    'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
    'Path', OrigPath) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Param) + ';', ';' + Uppercase(OrigPath) + ';') = 0;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  OrigPath, AppDir, NewPath: string;
  P: Integer;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if RegQueryStringValue(HKEY_LOCAL_MACHINE,
      'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
      'Path', OrigPath) then
    begin
      AppDir := ExpandConstant('{app}');
      P := Pos(';' + Uppercase(AppDir), Uppercase(OrigPath));
      if P > 0 then
      begin
        NewPath := Copy(OrigPath, 1, P - 1) + Copy(OrigPath, P + Length(AppDir) + 1, MaxInt);
        RegWriteExpandStringValue(HKEY_LOCAL_MACHINE,
          'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
          'Path', NewPath);
      end else
      begin
        P := Pos(Uppercase(AppDir) + ';', Uppercase(OrigPath));
        if P > 0 then
        begin
          NewPath := Copy(OrigPath, 1, P - 1) + Copy(OrigPath, P + Length(AppDir) + 1, MaxInt);
          RegWriteExpandStringValue(HKEY_LOCAL_MACHINE,
            'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
            'Path', NewPath);
        end;
      end;
    end;
  end;
end;

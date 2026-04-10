; Forge VCS Server Installer for Windows
; Installs forge-server, forge-web, forge CLI, UI assets, and config templates

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
SetupIconFile=forge.ico
UninstallDisplayIcon={app}\forge-server.exe

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
; Server binaries
Source: "{#ArtifactDir}\forge-server-windows\forge-server.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ArtifactDir}\forge-server-windows\forge-web.exe"; DestDir: "{app}"; Flags: ignoreversion

; CLI client (server admins need it too)
Source: "{#ArtifactDir}\forge-windows-client\forge.exe"; DestDir: "{app}"; Flags: ignoreversion

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
Name: "{group}\Edit Server Config"; Filename: "notepad.exe"; Parameters: """{app}\forge-server.toml"""
Name: "{group}\Edit Web Config"; Filename: "notepad.exe"; Parameters: """{app}\forge-web.toml"""
Name: "{group}\Start Forge Services"; Filename: "{app}\forge-server.exe"; Parameters: "service start"; WorkingDir: "{app}"; Comment: "Start the ForgeServer + ForgeWeb Windows services"
Name: "{group}\Stop Forge Services"; Filename: "{app}\forge-server.exe"; Parameters: "service stop"; WorkingDir: "{app}"; Comment: "Stop the ForgeServer + ForgeWeb Windows services"
Name: "{group}\Uninstall Forge Server"; Filename: "{uninstallexe}"

[Run]
; Register both binaries with the Windows Service Control Manager and
; start them. We point each `service install` at the absolute installed
; path of its config so the SCM-launched service binds the same TOML the
; user (or this installer) just laid down.
Filename: "{app}\forge-server.exe"; \
    Parameters: "--config ""{app}\forge-server.toml"" service install"; \
    StatusMsg: "Installing ForgeServer Windows service..."; \
    Flags: runhidden waituntilterminated; \
    Tasks: installServices

Filename: "{app}\forge-web.exe"; \
    Parameters: "--config ""{app}\forge-web.toml"" service install"; \
    StatusMsg: "Installing ForgeWeb Windows service..."; \
    Flags: runhidden waituntilterminated; \
    Tasks: installServices

Filename: "{app}\forge-server.exe"; \
    Parameters: "service start"; \
    StatusMsg: "Starting ForgeServer..."; \
    Flags: runhidden nowait; \
    Tasks: installServices

Filename: "{app}\forge-web.exe"; \
    Parameters: "service start"; \
    StatusMsg: "Starting ForgeWeb..."; \
    Flags: runhidden nowait; \
    Tasks: installServices

[UninstallRun]
; Stop and remove both services on uninstall. We use `runhidden
; runascurrentuser` so the SCM commands inherit the elevated context the
; uninstaller already runs in.
Filename: "{app}\forge-web.exe"; \
    Parameters: "service uninstall"; \
    Flags: runhidden waituntilterminated; \
    RunOnceId: "ForgeWebUninstallService"

Filename: "{app}\forge-server.exe"; \
    Parameters: "service uninstall"; \
    Flags: runhidden waituntilterminated; \
    RunOnceId: "ForgeServerUninstallService"

[Tasks]
Name: "installServices"; \
    Description: "Install and start ForgeServer + ForgeWeb as Windows services"; \
    GroupDescription: "Service installation:"; \
    Flags: checkedonce

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

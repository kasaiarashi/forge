; Forge VCS Client Installer for Windows
; Installs forge.exe CLI + optional Unreal Engine plugin

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif

#ifndef ArtifactDir
  #define ArtifactDir "..\..\artifacts"
#endif

#ifndef PluginDir
  #define PluginDir "..\..\plugin\ForgeSourceControl\Plugins\ForgeSourceControl"
#endif

[Setup]
AppName=Forge VCS
AppVersion={#AppVersion}
AppPublisher=Krishna Teja
AppPublisherURL=https://github.com/nicholasgasior/forge
DefaultDirName={autopf}\Forge
DefaultGroupName=Forge VCS
OutputBaseFilename=ForgeClient-Windows-x64-Setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesEnvironment=yes
PrivilegesRequired=admin
WizardStyle=modern
SetupIconFile=compiler:SetupClassicIcon.ico
UninstallDisplayIcon={app}\forge.exe

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
; Forge CLI binary
Source: "{#ArtifactDir}\forge-windows-client\forge.exe"; DestDir: "{app}"; Flags: ignoreversion

; UE Plugin files (only installed if user selects a UE path)
Source: "{#PluginDir}\ForgeSourceControl.uplugin"; DestDir: "{code:GetUEPluginDestDir}"; Flags: ignoreversion; Check: ShouldInstallPlugin
Source: "{#PluginDir}\Source\*"; DestDir: "{code:GetUEPluginDestDir}\Source"; Flags: ignoreversion recursesubdirs createallsubdirs; Check: ShouldInstallPlugin

[Registry]
; Add {app} to system PATH
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; \
    ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
    Check: NeedsAddPath(ExpandConstant('{app}'))

[Icons]
Name: "{group}\Forge Command Prompt"; Filename: "{cmd}"; Parameters: "/K set PATH={app};%PATH%"; WorkingDir: "{userdocs}"
Name: "{group}\Uninstall Forge"; Filename: "{uninstallexe}"

[Code]
var
  UEPage: TInputDirWizardPage;
  UEInstallPlugin: Boolean;
  UEDetectedPaths: TStringList;
  UEDetectedLabels: TStringList;
  SkipPluginCheckBox: TNewCheckBox;
  UEComboBox: TNewComboBox;

// Check if a path is already in the system PATH
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
  // Look for the path with leading and trailing semicolons
  Result := Pos(';' + Uppercase(Param) + ';', ';' + Uppercase(OrigPath) + ';') = 0;
end;

// Remove {app} from PATH on uninstall
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

// Detect UE installations from registry
procedure DetectUnrealEngines();
var
  SubKeys: TArrayOfString;
  I: Integer;
  InstallDir, VersionKey: string;
begin
  UEDetectedPaths := TStringList.Create;
  UEDetectedLabels := TStringList.Create;

  // Check registry for Epic Games Launcher installed engines
  if RegGetSubkeyNames(HKEY_LOCAL_MACHINE, 'SOFTWARE\EpicGames\Unreal Engine', SubKeys) then
  begin
    for I := 0 to GetArrayLength(SubKeys) - 1 do
    begin
      VersionKey := 'SOFTWARE\EpicGames\Unreal Engine\' + SubKeys[I];
      if RegQueryStringValue(HKEY_LOCAL_MACHINE, VersionKey, 'InstalledDirectory', InstallDir) then
      begin
        if DirExists(InstallDir) then
        begin
          UEDetectedLabels.Add('Unreal Engine ' + SubKeys[I] + ' - ' + InstallDir);
          UEDetectedPaths.Add(InstallDir);
        end;
      end;
    end;
  end;

  // Also scan common install locations
  for I := 0 to 9 do
  begin
    InstallDir := 'C:\Program Files\Epic Games\UE_5.' + IntToStr(I);
    if DirExists(InstallDir) then
    begin
      // Avoid duplicates
      if UEDetectedPaths.IndexOf(InstallDir) < 0 then
      begin
        UEDetectedLabels.Add('Unreal Engine 5.' + IntToStr(I) + ' - ' + InstallDir);
        UEDetectedPaths.Add(InstallDir);
      end;
    end;
  end;
end;

procedure OnSkipPluginClick(Sender: TObject);
begin
  UEComboBox.Enabled := not SkipPluginCheckBox.Checked;
  UEPage.Buttons[0].Enabled := not SkipPluginCheckBox.Checked;
  UEPage.Edits[0].Enabled := not SkipPluginCheckBox.Checked;
end;

procedure InitializeWizard();
var
  I: Integer;
  ComboLabel: TNewStaticText;
begin
  DetectUnrealEngines();

  // Create the UE plugin installation page
  UEPage := CreateInputDirPage(wpSelectDir,
    'Unreal Engine Plugin',
    'Install the Forge source control plugin for Unreal Engine.',
    'Select the Unreal Engine installation folder, or browse to locate it manually.' + #13#10 +
    'The plugin will be installed to Engine\Plugins\Marketplace\ForgeSourceControl\.',
    False, '');
  UEPage.Add('UE Installation Path:');

  // Add skip checkbox
  SkipPluginCheckBox := TNewCheckBox.Create(UEPage);
  SkipPluginCheckBox.Parent := UEPage.Surface;
  SkipPluginCheckBox.Caption := 'Skip Unreal Engine plugin installation';
  SkipPluginCheckBox.Top := UEPage.Edits[0].Top + UEPage.Edits[0].Height + 16;
  SkipPluginCheckBox.Left := UEPage.Edits[0].Left;
  SkipPluginCheckBox.Width := UEPage.SurfaceWidth;
  SkipPluginCheckBox.OnClick := @OnSkipPluginClick;

  // Add detected engines combo box if any found
  if UEDetectedPaths.Count > 0 then
  begin
    ComboLabel := TNewStaticText.Create(UEPage);
    ComboLabel.Parent := UEPage.Surface;
    ComboLabel.Caption := 'Detected Unreal Engine installations:';
    ComboLabel.Top := SkipPluginCheckBox.Top + SkipPluginCheckBox.Height + 16;
    ComboLabel.Left := UEPage.Edits[0].Left;

    UEComboBox := TNewComboBox.Create(UEPage);
    UEComboBox.Parent := UEPage.Surface;
    UEComboBox.Top := ComboLabel.Top + ComboLabel.Height + 4;
    UEComboBox.Left := UEPage.Edits[0].Left;
    UEComboBox.Width := UEPage.SurfaceWidth - UEPage.Edits[0].Left;
    UEComboBox.Style := csDropDownList;

    for I := 0 to UEDetectedLabels.Count - 1 do
      UEComboBox.Items.Add(UEDetectedLabels[I]);

    UEComboBox.ItemIndex := 0;

    // Pre-fill the path input with the first detected engine
    UEPage.Values[0] := UEDetectedPaths[0];
  end else
  begin
    // No engines detected — create a dummy combo so references don't fail
    UEComboBox := TNewComboBox.Create(UEPage);
    UEComboBox.Parent := UEPage.Surface;
    UEComboBox.Visible := False;
  end;
end;

// Update the path edit when user selects from combo box
function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = UEPage.ID then
  begin
    if not SkipPluginCheckBox.Checked then
    begin
      // Sync combo selection to path edit
      if (UEComboBox.ItemIndex >= 0) and (UEComboBox.ItemIndex < UEDetectedPaths.Count) then
        UEPage.Values[0] := UEDetectedPaths[UEComboBox.ItemIndex];

      // Validate the selected path
      if not DirExists(UEPage.Values[0]) then
      begin
        MsgBox('The selected Unreal Engine path does not exist. Please select a valid path or skip plugin installation.', mbError, MB_OK);
        Result := False;
        exit;
      end;

      // Verify it looks like a UE installation
      if not DirExists(UEPage.Values[0] + '\Engine') then
      begin
        MsgBox('The selected folder does not appear to be an Unreal Engine installation (no Engine subfolder found).', mbError, MB_OK);
        Result := False;
        exit;
      end;
    end;
  end;
end;

function ShouldInstallPlugin(): Boolean;
begin
  Result := not SkipPluginCheckBox.Checked;
end;

function GetUEPluginDestDir(Param: string): string;
begin
  Result := UEPage.Values[0] + '\Engine\Plugins\Marketplace\ForgeSourceControl';
end;

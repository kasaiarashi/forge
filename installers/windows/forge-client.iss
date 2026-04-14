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

#define FabMarketplaceURL "https://www.fab.com/listings/7cd90180-8c2f-4b64-a772-2f010cec0105"

[Setup]
AppName=Forge VCS
AppVersion={#AppVersion}
AppPublisher=Krishna Teja
AppPublisherURL=https://github.com/kasaiarashi/forge
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
SetupIconFile=forge.ico
UninstallDisplayIcon={app}\forge.exe

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
; Forge CLI binary
Source: "{#ArtifactDir}\forge-windows-client\forge.exe"; DestDir: "{app}"; Flags: ignoreversion

; UE Plugin files (only installed if user selects local install)
Source: "{#PluginDir}\ForgeSourceControl.uplugin"; DestDir: "{code:GetUEPluginDestDir}"; Flags: ignoreversion; Check: ShouldInstallPlugin
Source: "{#PluginDir}\Source\*"; DestDir: "{code:GetUEPluginDestDir}\Source"; Flags: ignoreversion recursesubdirs createallsubdirs; Check: ShouldInstallPlugin
Source: "{#PluginDir}\Binaries\*"; DestDir: "{code:GetUEPluginDestDir}\Binaries"; Flags: ignoreversion recursesubdirs createallsubdirs; Check: ShouldInstallPlugin
Source: "{#PluginDir}\Resources\*"; DestDir: "{code:GetUEPluginDestDir}\Resources"; Flags: ignoreversion recursesubdirs createallsubdirs; Check: ShouldInstallPlugin

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
  UEPage: TWizardPage;
  RadioInstallLocal: TNewRadioButton;
  RadioFabMarketplace: TNewRadioButton;
  RadioSkipPlugin: TNewRadioButton;
  FabInfoLabel: TNewStaticText;
  UEDirEdit: TEdit;
  UEDirBrowseBtn: TNewButton;
  UEDirLabel: TNewStaticText;
  UEDetectedPaths: TStringList;
  UEDetectedLabels: TStringList;
  UEComboBox: TNewComboBox;
  ComboLabel: TNewStaticText;

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
      if UEDetectedPaths.IndexOf(InstallDir) < 0 then
      begin
        UEDetectedLabels.Add('Unreal Engine 5.' + IntToStr(I) + ' - ' + InstallDir);
        UEDetectedPaths.Add(InstallDir);
      end;
    end;
  end;
end;

procedure UpdatePluginControls();
var
  LocalSelected: Boolean;
  FabSelected: Boolean;
begin
  LocalSelected := RadioInstallLocal.Checked;
  FabSelected := RadioFabMarketplace.Checked;

  // Local install controls
  UEDirLabel.Enabled := LocalSelected;
  UEDirEdit.Enabled := LocalSelected;
  UEDirBrowseBtn.Enabled := LocalSelected;
  UEComboBox.Enabled := LocalSelected;
  ComboLabel.Enabled := LocalSelected;

  // Fab info label
  FabInfoLabel.Visible := FabSelected;
end;

procedure OnPluginRadioClick(Sender: TObject);
begin
  UpdatePluginControls();
end;

procedure OnBrowseClick(Sender: TObject);
var
  Dir: string;
begin
  Dir := UEDirEdit.Text;
  if BrowseForFolder('Select Unreal Engine installation folder:', Dir, False) then
    UEDirEdit.Text := Dir;
end;

procedure OnComboChange(Sender: TObject);
begin
  if (UEComboBox.ItemIndex >= 0) and (UEComboBox.ItemIndex < UEDetectedPaths.Count) then
    UEDirEdit.Text := UEDetectedPaths[UEComboBox.ItemIndex];
end;

procedure InitializeWizard();
var
  I, Y: Integer;
begin
  DetectUnrealEngines();

  // Create custom plugin page
  UEPage := CreateCustomPage(wpSelectDir,
    'Unreal Engine Plugin',
    'Choose how to install the Forge source control plugin for Unreal Engine.');

  Y := 0;

  // ── Radio 1: Install from this installer ──
  RadioInstallLocal := TNewRadioButton.Create(UEPage);
  RadioInstallLocal.Parent := UEPage.Surface;
  RadioInstallLocal.Caption := 'Install plugin from this installer';
  RadioInstallLocal.Top := Y;
  RadioInstallLocal.Left := 0;
  RadioInstallLocal.Width := UEPage.SurfaceWidth;
  RadioInstallLocal.Checked := True;
  RadioInstallLocal.OnClick := @OnPluginRadioClick;
  Y := Y + 24;

  // Detected engines combo
  ComboLabel := TNewStaticText.Create(UEPage);
  ComboLabel.Parent := UEPage.Surface;
  ComboLabel.Top := Y;
  ComboLabel.Left := 24;
  Y := Y + 18;

  UEComboBox := TNewComboBox.Create(UEPage);
  UEComboBox.Parent := UEPage.Surface;
  UEComboBox.Top := Y;
  UEComboBox.Left := 24;
  UEComboBox.Width := UEPage.SurfaceWidth - 24;
  UEComboBox.Style := csDropDownList;
  UEComboBox.OnChange := @OnComboChange;

  if UEDetectedPaths.Count > 0 then
  begin
    ComboLabel.Caption := 'Detected installations:';
    for I := 0 to UEDetectedLabels.Count - 1 do
      UEComboBox.Items.Add(UEDetectedLabels[I]);
    UEComboBox.ItemIndex := 0;
  end else
  begin
    ComboLabel.Caption := 'No Unreal Engine installations detected.';
    UEComboBox.Visible := False;
  end;
  Y := Y + 28;

  // UE dir input + browse
  UEDirLabel := TNewStaticText.Create(UEPage);
  UEDirLabel.Parent := UEPage.Surface;
  UEDirLabel.Caption := 'UE Installation Path:';
  UEDirLabel.Top := Y;
  UEDirLabel.Left := 24;
  Y := Y + 18;

  UEDirEdit := TEdit.Create(UEPage);
  UEDirEdit.Parent := UEPage.Surface;
  UEDirEdit.Top := Y;
  UEDirEdit.Left := 24;
  UEDirEdit.Width := UEPage.SurfaceWidth - 24 - 90;

  UEDirBrowseBtn := TNewButton.Create(UEPage);
  UEDirBrowseBtn.Parent := UEPage.Surface;
  UEDirBrowseBtn.Caption := 'Browse...';
  UEDirBrowseBtn.Top := Y - 2;
  UEDirBrowseBtn.Left := UEDirEdit.Left + UEDirEdit.Width + 8;
  UEDirBrowseBtn.Width := 80;
  UEDirBrowseBtn.OnClick := @OnBrowseClick;

  if UEDetectedPaths.Count > 0 then
    UEDirEdit.Text := UEDetectedPaths[0];

  Y := Y + 36;

  // ── Radio 2: Get from Fab Marketplace ──
  RadioFabMarketplace := TNewRadioButton.Create(UEPage);
  RadioFabMarketplace.Parent := UEPage.Surface;
  RadioFabMarketplace.Caption := 'Get plugin from Fab Marketplace';
  RadioFabMarketplace.Top := Y;
  RadioFabMarketplace.Left := 0;
  RadioFabMarketplace.Width := UEPage.SurfaceWidth;
  RadioFabMarketplace.OnClick := @OnPluginRadioClick;
  Y := Y + 24;

  FabInfoLabel := TNewStaticText.Create(UEPage);
  FabInfoLabel.Parent := UEPage.Surface;
  FabInfoLabel.Caption :=
    'The Fab Marketplace page will open in your browser after installation.' + #13#10 +
    'Install the plugin directly from there into your Unreal Engine.' + #13#10 +
    'This is recommended if you want automatic plugin updates via the Epic Games Launcher.';
  FabInfoLabel.Top := Y;
  FabInfoLabel.Left := 24;
  FabInfoLabel.Width := UEPage.SurfaceWidth - 24;
  FabInfoLabel.AutoSize := True;
  FabInfoLabel.Visible := False;
  Y := Y + 60;

  // ── Radio 3: Skip entirely ──
  RadioSkipPlugin := TNewRadioButton.Create(UEPage);
  RadioSkipPlugin.Parent := UEPage.Surface;
  RadioSkipPlugin.Caption := 'Skip plugin installation (CLI only)';
  RadioSkipPlugin.Top := Y;
  RadioSkipPlugin.Left := 0;
  RadioSkipPlugin.Width := UEPage.SurfaceWidth;
  RadioSkipPlugin.OnClick := @OnPluginRadioClick;
end;

// Update the path edit when user selects from combo box
function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = UEPage.ID then
  begin
    if RadioInstallLocal.Checked then
    begin
      if UEDirEdit.Text = '' then
      begin
        MsgBox('Please select an Unreal Engine installation path.', mbError, MB_OK);
        Result := False;
        exit;
      end;

      if not DirExists(UEDirEdit.Text) then
      begin
        MsgBox('The selected Unreal Engine path does not exist.', mbError, MB_OK);
        Result := False;
        exit;
      end;

      if not DirExists(UEDirEdit.Text + '\Engine') then
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
  Result := RadioInstallLocal.Checked;
end;

function GetUEPluginDestDir(Param: string): string;
begin
  if RadioInstallLocal.Checked then
    Result := UEDirEdit.Text + '\Engine\Plugins\Marketplace\ForgeSourceControl'
  else
    Result := ExpandConstant('{tmp}');  // dummy, won't be used
end;

// Open Fab Marketplace after install if selected
procedure CurStepChanged(CurStep: TSetupStep);
var
  ErrorCode: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    if RadioFabMarketplace.Checked then
    begin
      ShellExec('open', '{#FabMarketplaceURL}', '', '', SW_SHOWNORMAL, ewNoWait, ErrorCode);
    end;
  end;
end;

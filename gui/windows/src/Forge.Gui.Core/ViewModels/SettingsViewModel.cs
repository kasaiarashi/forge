using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Core.ViewModels;

public sealed partial class SettingsViewModel : ObservableObject
{
    private readonly IUiModeService _mode;
    private readonly IForgeClient _client;

    [ObservableProperty]
    private bool _advancedMode;

    [ObservableProperty]
    private string _serverUrl = string.Empty;

    [ObservableProperty]
    private string _username = string.Empty;

    [ObservableProperty]
    private string _token = string.Empty;

    [ObservableProperty]
    private string? _signInStatus;

    public SettingsViewModel(IUiModeService mode, IForgeClient client)
    {
        _mode = mode;
        _client = client;
        _advancedMode = mode.Current == UiMode.Advanced;
    }

    partial void OnAdvancedModeChanged(bool value)
    {
        _mode.SetMode(value ? UiMode.Advanced : UiMode.Simple);
    }

    [RelayCommand]
    public async Task SignInAsync()
    {
        SignInStatus = "Signing in…";
        try
        {
            await _client.LoginAsync(ServerUrl, Username, Token).ConfigureAwait(false);
            SignInStatus = $"Signed in as {Username}";
            Token = string.Empty;
        }
        catch (Exception ex)
        {
            SignInStatus = $"Failed: {ex.Message}";
        }
    }
}

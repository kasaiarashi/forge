using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Forge.Gui.Core.Models;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Core.ViewModels;

public sealed partial class CloneViewModel : ObservableObject
{
    private readonly IForgeClient _client;

    [ObservableProperty] private string _url = string.Empty;
    [ObservableProperty] private string _destination = string.Empty;
    [ObservableProperty] private string? _token;
    [ObservableProperty] private bool _isCloning;
    [ObservableProperty] private double _progressFraction;
    [ObservableProperty] private string? _progressStage;
    [ObservableProperty] private string? _errorMessage;
    [ObservableProperty] private CloneResult? _result;

    public CloneViewModel(IForgeClient client) { _client = client; }

    [RelayCommand]
    public async Task StartAsync(CancellationToken ct = default)
    {
        if (string.IsNullOrWhiteSpace(Url) || string.IsNullOrWhiteSpace(Destination)) return;
        IsCloning = true;
        ErrorMessage = null;
        ProgressFraction = 0;
        var progress = new Progress<CloneProgress>(p =>
        {
            ProgressStage = p.Stage;
            ProgressFraction = p.Total > 0 ? (double)p.Current / p.Total : 0.0;
        });
        try
        {
            Result = await _client.CloneAsync(Url, Destination, progress, ct).ConfigureAwait(false);
        }
        catch (Exception ex)
        {
            ErrorMessage = ex.Message;
        }
        finally
        {
            IsCloning = false;
        }
    }
}

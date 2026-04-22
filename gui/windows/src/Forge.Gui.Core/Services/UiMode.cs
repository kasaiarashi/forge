namespace Forge.Gui.Core.Services;

public enum UiMode { Simple, Advanced }

public interface IUiModeService
{
    UiMode Current { get; }
    event EventHandler<UiMode>? ModeChanged;
    void SetMode(UiMode mode);
}

public sealed class UiModeService : IUiModeService
{
    private UiMode _current = UiMode.Simple;

    public UiMode Current => _current;
    public event EventHandler<UiMode>? ModeChanged;

    public void SetMode(UiMode mode)
    {
        if (_current == mode) return;
        _current = mode;
        ModeChanged?.Invoke(this, mode);
    }
}

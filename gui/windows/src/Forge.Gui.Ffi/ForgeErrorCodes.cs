namespace Forge.Gui.Ffi;

/// <summary>
/// Public mirror of the FFI's <c>forge_status_t</c> numeric codes.
/// Keep in sync with <c>crates/forge-ffi/src/lib.rs</c>.
/// </summary>
public static class ForgeErrorCodes
{
    public const int Ok              = 0;
    public const int Io              = 1;
    public const int Arg             = 2;
    public const int Auth            = 3;
    public const int NotFound        = 4;
    public const int Conflict        = 5;
    public const int NotAWorkspace   = 6;
    public const int Internal        = 99;
}

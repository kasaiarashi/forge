namespace Forge.Gui.Ffi;

public sealed class ForgeFfiException : Exception
{
    public int Code { get; }

    public ForgeFfiException(int code, string message) : base(message)
    {
        Code = code;
    }
}

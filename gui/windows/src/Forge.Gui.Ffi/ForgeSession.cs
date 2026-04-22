using Microsoft.Win32.SafeHandles;

namespace Forge.Gui.Ffi;

/// <summary>
/// RAII handle for a <c>forge_session_t*</c>. Guarantees
/// <see cref="NativeMethods.ForgeSessionClose"/> is called exactly
/// once regardless of how the managed caller exits.
/// </summary>
public sealed class ForgeSession : SafeHandleZeroOrMinusOneIsInvalid
{
    private ForgeSession() : base(ownsHandle: true) { }

    public static ForgeSession Open(string workspacePath)
    {
        var err = default(NativeMethods.ForgeError);
        var raw = NativeMethods.ForgeSessionOpen(workspacePath, ref err);
        if (raw == IntPtr.Zero)
        {
            var msg = NativeMethods.TakeError(ref err) ?? "open failed (unknown)";
            throw new ForgeFfiException(err.Code, msg);
        }
        var session = new ForgeSession();
        session.SetHandle(raw);
        return session;
    }

    protected override bool ReleaseHandle()
    {
        if (handle != IntPtr.Zero)
            NativeMethods.ForgeSessionClose(handle);
        return true;
    }

    public IntPtr Raw => handle;
}

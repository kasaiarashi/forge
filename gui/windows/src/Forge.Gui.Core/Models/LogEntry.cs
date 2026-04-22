namespace Forge.Gui.Core.Models;

public sealed record LogEntry(
    string Hash,
    IReadOnlyList<string> ParentHashes,
    string AuthorName,
    string AuthorEmail,
    DateTimeOffset Timestamp,
    string Message)
{
    public string ShortHash => Hash.Length >= 8 ? Hash.Substring(0, 8) : Hash;

    // Commit-message convention mirrors git: first line is the subject,
    // optional blank line, then body paragraphs. We surface them
    // separately so the History row can emphasise the subject without
    // losing the rest.
    public string Subject
    {
        get
        {
            if (string.IsNullOrEmpty(Message)) return string.Empty;
            var nl = Message.IndexOf('\n');
            return (nl < 0 ? Message : Message.Substring(0, nl)).TrimEnd('\r');
        }
    }

    public string Body
    {
        get
        {
            if (string.IsNullOrEmpty(Message)) return string.Empty;
            var nl = Message.IndexOf('\n');
            if (nl < 0) return string.Empty;
            return Message.Substring(nl + 1).TrimStart('\r', '\n').TrimEnd();
        }
    }

    public bool HasBody => !string.IsNullOrWhiteSpace(Body);
}

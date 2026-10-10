using System.Diagnostics;

namespace TicketBoard.SelfCheck;

/// <summary>Проверки одной самопроверки: непрошедшая печатается сразу (с подробностями), в конце AssertAll роняет процесс
/// с их числом — так в выводе виден весь список, а не только первая.</summary>
internal sealed class CheckSet(string area)
{
    public int Failures { get; private set; }

    public void Check(string name, bool ok, Func<string>? details = null)
    {
        if (ok) return;
        Failures++;
        Console.Error.WriteLine($"  {area}, не прошло: {name}" + (details is null ? "" : $"\n    {details().Replace("\n", "\n    ")}"));
    }

    /// <summary>Проверку пришлось пропустить (поток задержался дольше паузы, о которой она) — сказать, а не промолчать:
    /// иначе пропущенная не отличалась бы от прошедшей.</summary>
    public void Skipped(string name, string why) => Console.Error.WriteLine($"  {area}, пропущено: {name} — {why}");

    public void AssertAll() => Debug.Assert(Failures == 0, $"{area}: не прошло проверок — {Failures} (список выше)");
}

/// <summary>Своя папка данных на время проверки: App.DataDir смотрит в неё; Dispose возвращает прежний и удаляет папку.</summary>
internal sealed class TempDataDir : IDisposable
{
    private readonly string _was = App.DataDir;

    public TempDataDir(string prefix)
    {
        Path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), $"{prefix}-{Guid.NewGuid():N}");
        Directory.CreateDirectory(Path);
        App.DataDir = Path;
    }

    public string Path { get; }

    public void Dispose()
    {
        App.DataDir = _was;
        // антивирус или индексатор Windows может держать только что записанный файл — уборка не повод ронять проверку
        try { Directory.Delete(Path, recursive: true); } catch (Exception e) when (e is IOException or UnauthorizedAccessException) { }
    }
}

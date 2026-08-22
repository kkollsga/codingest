namespace MyApp.Logging;

/// <summary>Formats log lines for the demo application.</summary>
public class Logger
{
    public const string Prefix = "app";

    /// <summary>Emits one formatted line.</summary>
    public static string Emit(string message)
    {
        return Prefix + ": " + message;
    }
}

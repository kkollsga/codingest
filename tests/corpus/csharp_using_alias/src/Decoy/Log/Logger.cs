namespace Log;

/// <summary>DECOY. A real, well-formed namespace whose name equals the alias
/// spelling used in src/App/Service.cs. Nothing may ever resolve here.</summary>
public class Logger
{
    public static string Emit(string message)
    {
        return "decoy: " + message;
    }
}

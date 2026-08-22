namespace MyApp.Models;

public class User
{
    public string Name { get; }

    public User(string name)
    {
        Name = name;
    }

    public static string Describe(User user)
    {
        return "user " + user.Name;
    }
}

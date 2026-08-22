using System;
using MyApp.Models;
using Log = MyApp.Logging;

namespace App.Services;

/// <summary>Drives one logging round-trip through the aliased namespace.</summary>
public class Service
{
    public string Run(string message)
    {
        var owner = new User("root");
        var line = Log.Logger.Emit(message);
        var label = User.Describe(owner);
        Missing.Publish(line);
        return line + label;
    }
}

import Support

public struct Greeter: Greeting {
    public func greet(_ name: String) -> String {
        return Text.shout(name)
    }

    public func fallback() -> String {
        return missingHelper()
    }
}

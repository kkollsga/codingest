package com.example;

/**
 * Shared base that owns the greeting prefix.
 */
public abstract class AbstractGreeter implements Greeter {
    // TODO: make the prefix configurable.
    protected static final String PREFIX = "hello";

    /**
     * The prefix every greeting line starts with.
     */
    protected String prefix() {
        return PREFIX;
    }
}

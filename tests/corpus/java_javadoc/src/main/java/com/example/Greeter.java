package com.example;

/**
 * Anything that can produce a greeting line.
 */
public interface Greeter {
    /**
     * Builds the greeting line for one recipient.
     */
    String greet(String who);
}

package com.example.util;

/**
 * Small string helpers shared by the greeters.
 */
public final class Text {
    /**
     * Upper-cases one value.
     */
    public static String shout(String value) {
        return value.toUpperCase();
    }

    // Not javadoc: this line comment must NOT become a docstring.
    static String quiet(String value) {
        return value.toLowerCase();
    }
}

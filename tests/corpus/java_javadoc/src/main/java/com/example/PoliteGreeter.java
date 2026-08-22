package com.example;

import com.acme.Formatter;
import com.example.util.Text;
import java.util.List;

/**
 * Greets loudly, by way of the shared text helper.
 */
public class PoliteGreeter extends AbstractGreeter {
    /**
     * Builds a loud greeting for one recipient.
     */
    @Override
    public String greet(String who) {
        String line = prefix() + " " + who;
        String loud = Text.shout(line);
        return Missing.decorate(loud);
    }
}

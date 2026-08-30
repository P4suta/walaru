package io.github.p4suta.walaru.agent;

import java.lang.instrument.Instrumentation;

/** Installs Walaru's single-pass Java/Kotlin class transformer. */
public final class WalaruAgent {
    private WalaruAgent() {}

    public static void premain(String ignoredArguments, Instrumentation instrumentation) {
        AgentBridge.initializeFromProperties();
        instrumentation.addTransformer(new WalaruTransformer(AgentConfiguration.fromProperties()), false);
    }
}

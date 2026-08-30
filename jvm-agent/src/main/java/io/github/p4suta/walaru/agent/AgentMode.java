package io.github.p4suta.walaru.agent;

enum AgentMode {
    FAST,
    FULL;

    static AgentMode parse(String value) {
        return "full".equalsIgnoreCase(value) ? FULL : FAST;
    }
}

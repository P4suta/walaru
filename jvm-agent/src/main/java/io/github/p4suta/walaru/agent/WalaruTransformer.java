package io.github.p4suta.walaru.agent;

import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;

final class WalaruTransformer implements ClassFileTransformer {
    private static final String OWN_PACKAGE = "io/github/p4suta/walaru/";
    private final AgentConfiguration configuration;

    WalaruTransformer(AgentConfiguration configuration) {
        this.configuration = configuration;
    }

    @Override
    public byte[] transform(
            Module module,
            ClassLoader loader,
            String className,
            Class<?> classBeingRedefined,
            ProtectionDomain protectionDomain,
            byte[] classfileBuffer) {
        if (className == null
                || className.startsWith(OWN_PACKAGE)
                || protectionDomain == null
                || protectionDomain.getCodeSource() == null) return null;
        try {
            if (!configuration.includes(java.nio.file.Path.of(protectionDomain.getCodeSource().getLocation().toURI()))) {
                return null;
            }
            return BytecodeInstrumenter.instrument(classfileBuffer, className, configuration.mode());
        } catch (Throwable failure) {
            AgentBridge.capabilityMissing("instrumentation", "failed to transform " + className);
            return null;
        }
    }
}

package io.github.p4suta.walaru.agent;

import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.FieldVisitor;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.commons.AdviceAdapter;
import org.objectweb.asm.commons.Method;
import java.util.HashSet;
import java.util.Set;

final class BytecodeInstrumenter {
    private BytecodeInstrumenter() {}

    static byte[] instrument(byte[] original, String owner, AgentMode mode) {
        ClassReader reader = new ClassReader(original);
        ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS) {
            @Override
            protected String getCommonSuperClass(String left, String right) {
                return "java/lang/Object";
            }
        };
        reader.accept(new InstrumentingClassVisitor(writer, owner, mode), ClassReader.EXPAND_FRAMES);
        return writer.toByteArray();
    }

    private static final class InstrumentingClassVisitor extends ClassVisitor {
        private final String owner;
        private final AgentMode mode;
        private String source = "Unknown.java";
        private SourceMap sourceMap = SourceMap.parse(null, source);
        private final Set<String> volatileFields = new HashSet<>();

        InstrumentingClassVisitor(ClassVisitor delegate, String owner, AgentMode mode) {
            super(Opcodes.ASM9, delegate);
            this.owner = owner;
            this.mode = mode;
        }

        @Override
        public void visitSource(String source, String debug) {
            this.source = source == null ? "Unknown.java" : source;
            sourceMap = SourceMap.parse(debug, this.source);
            super.visitSource(source, debug);
        }

        @Override
        public FieldVisitor visitField(
                int access, String name, String descriptor, String signature, Object value) {
            if ((access & Opcodes.ACC_VOLATILE) != 0) volatileFields.add(name + '\0' + descriptor);
            return super.visitField(access, name, descriptor, signature, value);
        }

        @Override
        public MethodVisitor visitMethod(
                int access, String name, String descriptor, String signature, String[] exceptions) {
            MethodVisitor delegate = super.visitMethod(access, name, descriptor, signature, exceptions);
            if (delegate == null || (access & (Opcodes.ACC_ABSTRACT | Opcodes.ACC_NATIVE)) != 0) return delegate;
            return new InstrumentingMethodVisitor(
                    delegate, access, name, descriptor, owner, sourceMap, source, mode, volatileFields);
        }
    }

    private static final class InstrumentingMethodVisitor extends AdviceAdapter {
        private static final Type BRIDGE = Type.getType(AgentBridge.class);
        private static final Method ENTER = new Method(
                "methodEntered", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;I[Ljava/lang/Object;)V");
        private static final Method EXIT = new Method(
                "methodExited", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;IZ)V");
        private static final Method LINE = new Method(
                "line", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/Object;[Ljava/lang/Object;)V");
        private static final Method CALL = new Method(
                "call", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;)V");
        private static final Method WRITE = new Method(
                "write", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/Object;Ljava/lang/Object;ZZ)V");
        private static final Method READ = new Method(
                "read", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/Object;Ljava/lang/Object;ZZ)V");
        private static final Method ARRAY_WRITE = new Method(
                "arrayWrite", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/Object;ILjava/lang/Object;)V");
        private static final Method ARRAY_READ = new Method(
                "arrayRead", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/Object;ILjava/lang/Object;)V");
        private static final Method MONITOR = new Method(
                "monitor", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;Ljava/lang/String;Ljava/lang/Object;)V");

        private final String methodName;
        private final String descriptor;
        private final String owner;
        private final SourceMap sourceMap;
        private final String fallbackSource;
        private final boolean isStatic;
        private final boolean synchronizedMethod;
        private final AgentMode mode;
        private final Set<String> volatileFields;
        private int line = 1;
        private String sourcePath;
        private boolean injecting;
        private boolean receiverInitialized;

        InstrumentingMethodVisitor(
                MethodVisitor delegate,
                int access,
                String name,
                String descriptor,
                String owner,
                SourceMap sourceMap,
                String fallbackSource,
                AgentMode mode,
                Set<String> volatileFields) {
            super(Opcodes.ASM9, delegate, access, name, descriptor);
            this.methodName = name;
            this.descriptor = descriptor;
            this.owner = owner;
            this.sourceMap = sourceMap;
            this.fallbackSource = fallbackSource;
            this.mode = mode;
            this.volatileFields = volatileFields;
            sourcePath = fallbackSource;
            isStatic = (access & Opcodes.ACC_STATIC) != 0;
            synchronizedMethod = (access & Opcodes.ACC_SYNCHRONIZED) != 0;
            receiverInitialized = isStatic || !name.equals("<init>");
        }

        @Override
        protected void onMethodEnter() {
            if (synchronizedMethod && mode == AgentMode.FULL) {
                bridge(() -> {
                    pushLocation();
                    push("enter");
                    push(isStatic ? "synchronizedStaticMethod" : "synchronizedMethod");
                    if (isStatic) visitInsn(Opcodes.ACONST_NULL);
                    else loadThis();
                    invokeStatic(BRIDGE, MONITOR);
                });
            }
            bridge(() -> {
                pushLocation();
                loadArgArray();
                invokeStatic(BRIDGE, ENTER);
            });
            receiverInitialized = true;
        }

        @Override
        protected void onMethodExit(int opcode) {
            bridge(() -> {
                pushLocation();
                push(opcode == Opcodes.ATHROW);
                invokeStatic(BRIDGE, EXIT);
            });
            if (synchronizedMethod && mode == AgentMode.FULL) {
                bridge(() -> {
                    pushLocation();
                    push("exit");
                    push(isStatic ? "synchronizedStaticMethod" : "synchronizedMethod");
                    if (isStatic) visitInsn(Opcodes.ACONST_NULL);
                    else loadThis();
                    invokeStatic(BRIDGE, MONITOR);
                });
            }
        }

        @Override
        public void visitLineNumber(int bytecodeLine, Label start) {
            SourceMap.Position position = sourceMap.position(bytecodeLine);
            line = position.line();
            sourcePath = position.path();
            super.visitLineNumber(bytecodeLine, start);
            bridge(() -> {
                pushLocation();
                if (isStatic || !receiverInitialized) visitInsn(Opcodes.ACONST_NULL);
                else loadThis();
                loadArgArray();
                invokeStatic(BRIDGE, LINE);
            });
        }

        @Override
        public void visitMethodInsn(int opcode, String targetOwner, String name, String targetDescriptor, boolean isInterface) {
            if (!injecting) {
                bridge(() -> {
                    pushLocation();
                    push(targetOwner);
                    push(name);
                    push(targetDescriptor);
                    invokeStatic(BRIDGE, CALL);
                });
            }
            if (!injecting && mode == AgentMode.FULL
                    && rewriteDeterministicInput(opcode, targetOwner, name, targetDescriptor)) {
                return;
            }
            super.visitMethodInsn(opcode, targetOwner, name, targetDescriptor, isInterface);
        }

        private boolean rewriteDeterministicInput(
                int opcode, String targetOwner, String name, String targetDescriptor) {
            if (opcode == Opcodes.INVOKESTATIC
                    && targetOwner.equals("java/nio/file/Files")
                    && Boolean.getBoolean("walaru.captureFileIo")) {
                String bridgeMethod = switch (name + targetDescriptor) {
                    case "readAllBytes(Ljava/nio/file/Path;)[B" -> "fileReadAllBytes";
                    case "readString(Ljava/nio/file/Path;)Ljava/lang/String;" -> "fileReadString";
                    case "readString(Ljava/nio/file/Path;Ljava/nio/charset/Charset;)Ljava/lang/String;" ->
                            "fileReadString";
                    default -> null;
                };
                if (bridgeMethod != null) {
                    super.visitMethodInsn(
                            Opcodes.INVOKESTATIC,
                            BRIDGE.getInternalName(),
                            bridgeMethod,
                            targetDescriptor,
                            false);
                    return true;
                }
            }
            if (opcode == Opcodes.INVOKESTATIC
                    && targetOwner.equals("java/lang/System")
                    && name.equals("currentTimeMillis")
                    && targetDescriptor.equals("()J")) {
                super.visitMethodInsn(
                        Opcodes.INVOKESTATIC, BRIDGE.getInternalName(), "currentTimeMillis", "()J", false);
                return true;
            }
            if (opcode == Opcodes.INVOKESTATIC
                    && targetOwner.equals("java/lang/System")
                    && name.equals("nanoTime")
                    && targetDescriptor.equals("()J")) {
                super.visitMethodInsn(Opcodes.INVOKESTATIC, BRIDGE.getInternalName(), "nanoTime", "()J", false);
                return true;
            }
            if (opcode == Opcodes.INVOKESTATIC
                    && targetOwner.equals("java/util/UUID")
                    && name.equals("randomUUID")
                    && targetDescriptor.equals("()Ljava/util/UUID;")) {
                super.visitMethodInsn(
                        Opcodes.INVOKESTATIC,
                        BRIDGE.getInternalName(),
                        "randomUuid",
                        "()Ljava/util/UUID;",
                        false);
                return true;
            }
            if (opcode == Opcodes.INVOKESTATIC
                    && targetOwner.equals("java/lang/Math")
                    && name.equals("random")
                    && targetDescriptor.equals("()D")) {
                super.visitMethodInsn(Opcodes.INVOKESTATIC, BRIDGE.getInternalName(), "mathRandom", "()D", false);
                return true;
            }
            if (opcode == Opcodes.INVOKEVIRTUAL
                    && isJavaRandom(targetOwner)
                    && rewriteJavaRandom(name, targetDescriptor)) return true;
            if (opcode == Opcodes.INVOKESTATIC && targetDescriptor.startsWith("()")) {
                String bridgeMethod = switch (targetOwner + '#' + name + targetDescriptor) {
                    case "java/time/Instant#now()Ljava/time/Instant;" -> "instantNow";
                    case "java/time/LocalDate#now()Ljava/time/LocalDate;" -> "localDateNow";
                    case "java/time/LocalDateTime#now()Ljava/time/LocalDateTime;" -> "localDateTimeNow";
                    default -> null;
                };
                if (bridgeMethod != null) {
                    super.visitMethodInsn(
                            Opcodes.INVOKESTATIC,
                            BRIDGE.getInternalName(),
                            bridgeMethod,
                            targetDescriptor,
                            false);
                    return true;
                }
            }
            return false;
        }

        private static boolean isJavaRandom(String owner) {
            return owner.equals("java/util/Random")
                    || owner.equals("java/security/SecureRandom")
                    || owner.equals("java/util/concurrent/ThreadLocalRandom");
        }

        private boolean rewriteJavaRandom(String name, String descriptor) {
            String bridgeMethod = switch (name + descriptor) {
                case "nextInt()I", "nextInt(I)I" -> "randomNextInt";
                case "nextLong()J", "nextLong(J)J" -> "randomNextLong";
                case "nextBoolean()Z" -> "randomNextBoolean";
                case "nextFloat()F" -> "randomNextFloat";
                case "nextDouble()D" -> "randomNextDouble";
                case "nextGaussian()D" -> "randomNextGaussian";
                case "nextBytes([B)V" -> "randomNextBytes";
                default -> null;
            };
            if (bridgeMethod == null) return false;
            String staticDescriptor = "(Ljava/util/Random;" + descriptor.substring(1);
            super.visitMethodInsn(
                    Opcodes.INVOKESTATIC,
                    BRIDGE.getInternalName(),
                    bridgeMethod,
                    staticDescriptor,
                    false);
            return true;
        }

        @Override
        public void visitFieldInsn(int opcode, String targetOwner, String name, String fieldDescriptor) {
            if (!injecting && mode == AgentMode.FULL
                    && (opcode == Opcodes.GETFIELD || opcode == Opcodes.GETSTATIC)) {
                instrumentFieldRead(opcode, targetOwner, name, fieldDescriptor);
                return;
            }
            if (!injecting && (opcode == Opcodes.PUTFIELD || opcode == Opcodes.PUTSTATIC)) {
                Type valueType = Type.getType(fieldDescriptor);
                boolean captureValue = mode == AgentMode.FULL
                        && (opcode == Opcodes.PUTSTATIC || receiverInitialized);
                boolean volatileField = targetOwner.equals(owner)
                        && volatileFields.contains(name + '\0' + fieldDescriptor);
                boolean staticField = opcode == Opcodes.PUTSTATIC;
                int valueLocal = -1;
                int receiverLocal = -1;
                if (captureValue) {
                    valueLocal = newLocal(valueType);
                    storeLocal(valueLocal, valueType);
                    if (opcode == Opcodes.PUTFIELD) {
                        Type receiverType = Type.getObjectType(targetOwner);
                        receiverLocal = newLocal(receiverType);
                        storeLocal(receiverLocal, receiverType);
                    }
                }
                int capturedValueLocal = valueLocal;
                int capturedReceiverLocal = receiverLocal;
                bridge(() -> {
                    pushLocation();
                    push(targetOwner);
                    push(name);
                    push(fieldDescriptor);
                    if (capturedReceiverLocal < 0) visitInsn(Opcodes.ACONST_NULL);
                    else loadLocal(capturedReceiverLocal);
                    if (capturedValueLocal < 0) visitInsn(Opcodes.ACONST_NULL);
                    else {
                        loadLocal(capturedValueLocal, valueType);
                        box(valueType);
                    }
                    push(volatileField);
                    push(staticField);
                    invokeStatic(BRIDGE, WRITE);
                });
                if (captureValue) {
                    if (receiverLocal >= 0) loadLocal(receiverLocal);
                    loadLocal(valueLocal, valueType);
                }
            }
            super.visitFieldInsn(opcode, targetOwner, name, fieldDescriptor);
        }

        private void instrumentFieldRead(
                int opcode, String targetOwner, String name, String fieldDescriptor) {
            Type valueType = Type.getType(fieldDescriptor);
            boolean staticField = opcode == Opcodes.GETSTATIC;
            boolean volatileField = targetOwner.equals(owner)
                    && volatileFields.contains(name + '\0' + fieldDescriptor);
            int receiverLocal = -1;
            if (!staticField) {
                Type receiverType = Type.getObjectType(targetOwner);
                receiverLocal = newLocal(receiverType);
                storeLocal(receiverLocal, receiverType);
                loadLocal(receiverLocal, receiverType);
            }
            super.visitFieldInsn(opcode, targetOwner, name, fieldDescriptor);
            int valueLocal = newLocal(valueType);
            storeLocal(valueLocal, valueType);
            int capturedReceiver = receiverLocal;
            bridge(() -> {
                pushLocation();
                push(targetOwner);
                push(name);
                push(fieldDescriptor);
                if (capturedReceiver < 0) visitInsn(Opcodes.ACONST_NULL);
                else loadLocal(capturedReceiver);
                loadLocal(valueLocal, valueType);
                box(valueType);
                push(volatileField);
                push(staticField);
                invokeStatic(BRIDGE, READ);
            });
            loadLocal(valueLocal, valueType);
        }

        @Override
        public void visitInsn(int opcode) {
            if (!injecting && mode == AgentMode.FULL && isArrayStore(opcode)) {
                instrumentArrayStore(opcode);
                return;
            }
            if (!injecting && mode == AgentMode.FULL && isArrayLoad(opcode)) {
                instrumentArrayLoad(opcode);
                return;
            }
            if (!injecting && mode == AgentMode.FULL
                    && (opcode == Opcodes.MONITORENTER || opcode == Opcodes.MONITOREXIT)) {
                int monitorLocal = newLocal(Type.getType(Object.class));
                storeLocal(monitorLocal);
                bridge(() -> {
                    pushLocation();
                    push(opcode == Opcodes.MONITORENTER ? "enter" : "exit");
                    push("block");
                    loadLocal(monitorLocal);
                    invokeStatic(BRIDGE, MONITOR);
                });
                loadLocal(monitorLocal);
            }
            super.visitInsn(opcode);
        }

        private void instrumentArrayStore(int opcode) {
            Type valueType = arrayValueType(opcode);
            int valueLocal = newLocal(valueType);
            storeLocal(valueLocal, valueType);
            int indexLocal = newLocal(Type.INT_TYPE);
            storeLocal(indexLocal);
            int arrayLocal = newLocal(Type.getType(Object.class));
            storeLocal(arrayLocal);
            bridge(() -> {
                pushLocation();
                loadLocal(arrayLocal);
                loadLocal(indexLocal);
                loadLocal(valueLocal, valueType);
                box(valueType);
                invokeStatic(BRIDGE, ARRAY_WRITE);
            });
            loadLocal(arrayLocal);
            loadLocal(indexLocal);
            loadLocal(valueLocal, valueType);
            super.visitInsn(opcode);
        }

        private void instrumentArrayLoad(int opcode) {
            int indexLocal = newLocal(Type.INT_TYPE);
            storeLocal(indexLocal);
            int arrayLocal = newLocal(Type.getType(Object.class));
            storeLocal(arrayLocal);
            loadLocal(arrayLocal);
            loadLocal(indexLocal);
            super.visitInsn(opcode);
            Type valueType = arrayValueType(opcode);
            int valueLocal = newLocal(valueType);
            storeLocal(valueLocal, valueType);
            bridge(() -> {
                pushLocation();
                loadLocal(arrayLocal);
                loadLocal(indexLocal);
                loadLocal(valueLocal, valueType);
                box(valueType);
                invokeStatic(BRIDGE, ARRAY_READ);
            });
            loadLocal(valueLocal, valueType);
        }

        private static boolean isArrayStore(int opcode) {
            return opcode >= Opcodes.IASTORE && opcode <= Opcodes.SASTORE;
        }

        private static boolean isArrayLoad(int opcode) {
            return opcode >= Opcodes.IALOAD && opcode <= Opcodes.SALOAD;
        }

        private static Type arrayValueType(int opcode) {
            return switch (opcode) {
                case Opcodes.LALOAD, Opcodes.LASTORE -> Type.LONG_TYPE;
                case Opcodes.FALOAD, Opcodes.FASTORE -> Type.FLOAT_TYPE;
                case Opcodes.DALOAD, Opcodes.DASTORE -> Type.DOUBLE_TYPE;
                case Opcodes.AALOAD, Opcodes.AASTORE -> Type.getType(Object.class);
                default -> Type.INT_TYPE;
            };
        }

        private void pushLocation() {
            push(owner);
            push(methodName);
            push(descriptor);
            push(sourcePath == null ? fallbackSource : sourcePath);
            push(Math.max(1, line));
        }

        private void bridge(Runnable emitter) {
            boolean previous = injecting;
            injecting = true;
            try {
                emitter.run();
            } finally {
                injecting = previous;
            }
        }
    }
}

package demo;

import java.util.Random;
import java.util.UUID;

public final class MavenClock {
    public String sample() {
        return System.currentTimeMillis() + ":" + UUID.randomUUID() + ":" + new Random().nextInt(100);
    }
}

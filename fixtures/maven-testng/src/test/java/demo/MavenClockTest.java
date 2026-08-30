package demo;

import org.testng.Assert;
import org.testng.annotations.Test;

public final class MavenClockTest {
    @Test
    public void recordsMavenAndTestNg() {
        Assert.assertEquals(new MavenClock().sample().split(":" ).length, 3);
    }
}

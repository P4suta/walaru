package example;

import io.github.p4suta.walaru.Walaru;
import java.util.Map;

public final class BinarySearch {
    private BinarySearch() {}

    public static int find(int[] values, int target, String apiToken) {
        try (var span = Walaru.span("binary search").capture("target", target)) {
            int low = 0;
            int high = values.length - 1;
            while (low < high) { // Intentional bug: the final candidate is never checked.
                int middle = Walaru.capture("middle", (low + high) >>> 1);
                Walaru.checkpoint("partition", Map.of("low", low, "high", high));
                if (values[middle] < target) {
                    low = middle + 1;
                } else if (values[middle] > target) {
                    high = middle - 1;
                } else {
                    return middle;
                }
            }
            Walaru.captureRedacted("apiToken", apiToken);
            return -1;
        }
    }
}

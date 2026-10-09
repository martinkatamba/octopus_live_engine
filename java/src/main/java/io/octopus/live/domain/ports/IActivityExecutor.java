package io.octopus.live.domain.ports;

import java.util.Map;
import java.util.function.Function;

public interface IActivityExecutor {
    Map<String, Object> executeActivity(String activityName, Map<String, Object> input);
    void registerActivity(String activityName, Function<Map<String, Object>, Map<String, Object>> handler);
    boolean hasActivity(String activityName);
}

package io.octopus.live.adapters.outbound;

import io.octopus.live.domain.ports.IActivityExecutor;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Function;

public class CallableActivityExecutor implements IActivityExecutor {

    private final ConcurrentHashMap<String, Function<Map<String, Object>, Map<String, Object>>> activities = new ConcurrentHashMap<>();

    @Override
    public void registerActivity(String activityName, Function<Map<String, Object>, Map<String, Object>> handler) {
        activities.put(activityName, handler);
    }

    @Override
    public boolean hasActivity(String activityName) {
        return activities.containsKey(activityName);
    }

    @Override
    public Map<String, Object> executeActivity(String activityName, Map<String, Object> input) {
        Function<Map<String, Object>, Map<String, Object>> handler = activities.get(activityName);
        if (handler == null) {
            throw new IllegalArgumentException("Activity handler '" + activityName + "' is not registered in Live Engine.");
        }
        return handler.apply(input);
    }
}

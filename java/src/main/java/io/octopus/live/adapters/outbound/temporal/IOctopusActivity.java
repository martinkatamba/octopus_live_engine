package io.octopus.live.adapters.outbound.temporal;

import io.temporal.activity.ActivityInterface;
import io.temporal.activity.ActivityMethod;
import java.util.Map;

@ActivityInterface
public interface IOctopusActivity {

    @ActivityMethod
    Map<String, Object> executeActivity(String activityName, Map<String, Object> payload);
}

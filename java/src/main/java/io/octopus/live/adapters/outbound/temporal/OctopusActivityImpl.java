package io.octopus.live.adapters.outbound.temporal;

import io.octopus.live.domain.ports.IActivityExecutor;
import java.util.Collections;
import java.util.Map;

public class OctopusActivityImpl implements IOctopusActivity {

    private final IActivityExecutor localExecutor;

    public OctopusActivityImpl(IActivityExecutor localExecutor) {
        this.localExecutor = localExecutor;
    }

    @Override
    public Map<String, Object> executeActivity(String activityName, Map<String, Object> payload) {
        if (localExecutor != null && localExecutor.hasActivity(activityName)) {
            Map<String, Object> res = localExecutor.executeActivity(activityName, payload);
            return res != null ? res : Collections.emptyMap();
        }
        return Collections.singletonMap("status", "SUCCESS");
    }
}

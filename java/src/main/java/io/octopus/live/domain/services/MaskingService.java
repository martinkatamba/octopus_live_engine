package io.octopus.live.domain.services;

import java.util.*;

public class MaskingService {

    @SuppressWarnings("unchecked")
    public static Map<String, Object> maskPayload(
            Map<String, Object> payload,
            Collection<String> maskFields,
            String maskValue
    ) {
        if (payload == null) {
            return new HashMap<>();
        }
        Set<String> fieldsSet = new TreeSet<>(String.CASE_INSENSITIVE_ORDER);
        if (maskFields != null) {
            for (String f : maskFields) {
                if (f != null && !f.trim().isEmpty()) {
                    fieldsSet.add(f.trim());
                }
            }
        }
        if (fieldsSet.isEmpty()) {
            return new HashMap<>(payload);
        }

        String effectiveMaskValue = (maskValue != null && !maskValue.isEmpty()) ? maskValue : "********";
        return (Map<String, Object>) maskValueInternal(payload, fieldsSet, effectiveMaskValue);
    }

    @SuppressWarnings("unchecked")
    private static Object maskValueInternal(
            Object value,
            Set<String> maskFields,
            String maskValue
    ) {
        if (value == null) {
            return null;
        }

        if (value instanceof Map) {
            Map<?, ?> sourceMap = (Map<?, ?>) value;
            Map<String, Object> result = new LinkedHashMap<>();
            for (Map.Entry<?, ?> entry : sourceMap.entrySet()) {
                String keyStr = entry.getKey() != null ? entry.getKey().toString() : "";
                if (maskFields.contains(keyStr)) {
                    result.put(keyStr, maskValue);
                } else {
                    result.put(keyStr, maskValueInternal(entry.getValue(), maskFields, maskValue));
                }
            }
            return result;
        }

        if (value instanceof Collection) {
            Collection<?> sourceCol = (Collection<?>) value;
            List<Object> result = new ArrayList<>(sourceCol.size());
            for (Object item : sourceCol) {
                result.add(maskValueInternal(item, maskFields, maskValue));
            }
            return result;
        }

        if (value.getClass().isArray()) {
            int length = java.lang.reflect.Array.getLength(value);
            List<Object> result = new ArrayList<>(length);
            for (int i = 0; i < length; i++) {
                result.add(maskValueInternal(java.lang.reflect.Array.get(value, i), maskFields, maskValue));
            }
            return result;
        }

        return value;
    }
}

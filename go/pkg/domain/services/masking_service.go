package services

import (
	"strings"
)

// MaskPayload creates a deep copy of payload and replaces values for any keys matching
// maskFields (case-insensitive) with maskValue (defaulting to "********").
func MaskPayload(payload map[string]interface{}, maskFields []string, maskValue string) map[string]interface{} {
	if payload == nil {
		return nil
	}
	if len(maskFields) == 0 {
		return copyMapRecursive(payload)
	}
	if maskValue == "" {
		maskValue = "********"
	}

	targets := make(map[string]bool, len(maskFields))
	for _, f := range maskFields {
		trimmed := strings.ToLower(strings.TrimSpace(f))
		if trimmed != "" {
			targets[trimmed] = true
		}
	}

	result := make(map[string]interface{}, len(payload))
	for k, v := range payload {
		kLower := strings.ToLower(strings.TrimSpace(k))
		if targets[kLower] {
			result[k] = maskValue
		} else {
			result[k] = sanitizeValue(v, targets, maskValue)
		}
	}
	return result
}

func sanitizeValue(v interface{}, targets map[string]bool, maskValue string) interface{} {
	if v == nil {
		return nil
	}

	switch val := v.(type) {
	case map[string]interface{}:
		sanitized := make(map[string]interface{}, len(val))
		for k, item := range val {
			kLower := strings.ToLower(strings.TrimSpace(k))
			if targets[kLower] {
				sanitized[k] = maskValue
			} else {
				sanitized[k] = sanitizeValue(item, targets, maskValue)
			}
		}
		return sanitized

	case map[interface{}]interface{}:
		sanitized := make(map[string]interface{}, len(val))
		for k, item := range val {
			kStr := strings.TrimSpace(strings.ToLower(toString(k)))
			originalKey := toString(k)
			if targets[kStr] {
				sanitized[originalKey] = maskValue
			} else {
				sanitized[originalKey] = sanitizeValue(item, targets, maskValue)
			}
		}
		return sanitized

	case []interface{}:
		sanitizedList := make([]interface{}, len(val))
		for i, item := range val {
			sanitizedList[i] = sanitizeValue(item, targets, maskValue)
		}
		return sanitizedList

	case []map[string]interface{}:
		sanitizedList := make([]map[string]interface{}, len(val))
		for i, item := range val {
			if m, ok := sanitizeValue(item, targets, maskValue).(map[string]interface{}); ok {
				sanitizedList[i] = m
			} else {
				sanitizedList[i] = item
			}
		}
		return sanitizedList

	default:
		return v
	}
}

func copyMapRecursive(m map[string]interface{}) map[string]interface{} {
	if m == nil {
		return nil
	}
	out := make(map[string]interface{}, len(m))
	for k, v := range m {
		switch val := v.(type) {
		case map[string]interface{}:
			out[k] = copyMapRecursive(val)
		case []interface{}:
			copiedSlice := make([]interface{}, len(val))
			for i, elem := range val {
				if elemMap, ok := elem.(map[string]interface{}); ok {
					copiedSlice[i] = copyMapRecursive(elemMap)
				} else {
					copiedSlice[i] = elem
				}
			}
			out[k] = copiedSlice
		default:
			out[k] = v
		}
	}
	return out
}

func toString(v interface{}) string {
	if v == nil {
		return ""
	}
	if s, ok := v.(string); ok {
		return s
	}
	return ""
}

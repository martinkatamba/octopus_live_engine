package services

import (
	"encoding/json"
	"fmt"
	"reflect"
	"strings"
)

// MaskPayload creates a deep copy of payload and replaces values for any keys matching
// maskFields (case-insensitive) with maskValue (defaulting to "********").
// Typed values (structs, typed maps and slices) are converted to plain JSON
// shapes first, so their fields are masked and the copy shares no memory
// with the original.
func MaskPayload(payload map[string]interface{}, maskFields []string, maskValue string) map[string]interface{} {
	if payload == nil {
		return nil
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
		if isMaskTarget(k, targets) {
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
			if isMaskTarget(k, targets) {
				sanitized[k] = maskValue
			} else {
				sanitized[k] = sanitizeValue(item, targets, maskValue)
			}
		}
		return sanitized

	case map[interface{}]interface{}:
		sanitized := make(map[string]interface{}, len(val))
		for k, item := range val {
			originalKey := toString(k)
			if isMaskTarget(originalKey, targets) {
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
		if n, ok := normalize(v); ok {
			return sanitizeValue(n, targets, maskValue)
		}
		return v
	}
}

// normalize converts typed composite values into plain JSON shapes
// (map[string]interface{}, []interface{}, scalars). Scalars and []byte are
// returned unchanged with ok=false.
func normalize(v interface{}) (interface{}, bool) {
	rv := reflect.ValueOf(v)
	switch rv.Kind() {
	case reflect.Struct, reflect.Map, reflect.Array:
	case reflect.Slice:
		if rv.Type().Elem().Kind() == reflect.Uint8 {
			return v, false
		}
	case reflect.Ptr, reflect.Interface:
		if rv.IsNil() {
			return nil, true
		}
	default:
		return v, false
	}
	b, err := json.Marshal(v)
	if err != nil {
		return fmt.Sprintf("<unserialisable %T>", v), true
	}
	var out interface{}
	if err := json.Unmarshal(b, &out); err != nil {
		return fmt.Sprintf("<unserialisable %T>", v), true
	}
	return out, true
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

// isMaskTarget matches a key against the mask fields case-insensitively. The
// copy of a request field the engine injects ("_injected_<field>") is masked
// like the field itself.
func isMaskTarget(key string, targets map[string]bool) bool {
	k := strings.ToLower(strings.TrimSpace(key))
	return targets[k] || targets[strings.TrimPrefix(k, "_injected_")]
}

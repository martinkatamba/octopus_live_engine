using System.Collections;

namespace Octopus.LiveEngine.Domain.Services;

public static class MaskingService
{
    public static Dictionary<string, object> MaskPayload(
        Dictionary<string, object>? payload,
        IEnumerable<string>? maskFields,
        string maskValue = "********"
    )
    {
        if (payload == null) return new Dictionary<string, object>();
        var fieldsSet = new HashSet<string>(maskFields ?? Enumerable.Empty<string>(), StringComparer.OrdinalIgnoreCase);
        if (fieldsSet.Count == 0) return new Dictionary<string, object>(payload);

        return (Dictionary<string, object>)MaskValueInternal(payload, fieldsSet, maskValue)!;
    }

    public static object? MaskValueInternal(
        object? value,
        HashSet<string> maskFields,
        string maskValue
    )
    {
        if (value == null) return null;

        if (value is IDictionary dict)
        {
            var result = new Dictionary<string, object?>();
            foreach (DictionaryEntry entry in dict)
            {
                var keyStr = entry.Key?.ToString() ?? string.Empty;
                if (maskFields.Contains(keyStr))
                {
                    result[keyStr] = maskValue;
                }
                else
                {
                    result[keyStr] = MaskValueInternal(entry.Value, maskFields, maskValue);
                }
            }
            return result;
        }

        if (value is string || value is ValueType)
        {
            return value;
        }

        if (value is IEnumerable enumerable)
        {
            var list = new List<object?>();
            foreach (var item in enumerable)
            {
                list.Add(MaskValueInternal(item, maskFields, maskValue));
            }
            return list;
        }

        return value;
    }
}

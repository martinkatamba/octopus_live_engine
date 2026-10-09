package io.octopus.live.domain.services.middleware;

import java.util.List;
import java.util.regex.Pattern;

public class RouteMatcher {

    public static boolean matches(String path, String pattern) {
        if (pattern == null || path == null) return false;
        String cleanPath = path.endsWith("/") && path.length() > 1 ? path.substring(0, path.length() - 1) : path;
        String cleanPat = pattern.endsWith("/") && pattern.length() > 1 ? pattern.substring(0, pattern.length() - 1) : pattern;

        if (cleanPath.equals(cleanPat)) return true;

        if (pattern.endsWith("*")) {
            String prefix = pattern.substring(0, pattern.length() - 1);
            if (path.startsWith(prefix)) return true;
        }

        if (pattern.contains("*") || pattern.contains("?")) {
            String regex = "^" + pattern
                    .replace(".", "\\.")
                    .replace("?", ".")
                    .replace("*", ".*") + "$";
            try {
                if (Pattern.matches(regex, path) || Pattern.matches(regex, cleanPath)) {
                    return true;
                }
            } catch (Exception ignored) {}
        }

        return false;
    }

    public static boolean shouldSkip(String path, List<String> excludes, List<String> includes) {
        if (includes != null && !includes.isEmpty()) {
            boolean included = false;
            for (String inc : includes) {
                if (matches(path, inc)) {
                    included = true;
                    break;
                }
            }
            if (!included) return true;
        }

        if (excludes != null && !excludes.isEmpty()) {
            for (String exc : excludes) {
                if (matches(path, exc)) {
                    return true;
                }
            }
        }

        return false;
    }
}

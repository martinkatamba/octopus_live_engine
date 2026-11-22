using System.Net;
using Microsoft.AspNetCore.Http;

namespace Octopus.LiveEngine;

/// <summary>Who may use the tooling routes: the visualizer, job history, timelines, live streams, diagrams and the API docs.</summary>
public enum ToolingAccess
{
    /// <summary>The default: clients on this machine only, unless an API key is configured (then the key is required).</summary>
    LocalOnly,
    /// <summary>The engine's API key is required; with no key configured, every request is refused.</summary>
    ApiKey,
    /// <summary>Anyone (the behaviour before 2026-11-22).</summary>
    Open,
    /// <summary>The tooling routes answer 404.</summary>
    Disabled,
}

/// <summary>Decides whether a request may use the tooling routes; it replaces the <see cref="EngineOptions.Tooling"/> rule.</summary>
public delegate bool ToolingAuthorizer(HttpContext context);

/// <summary>How the engine serves its routes. The defaults are the secure ones.</summary>
public sealed class EngineOptions
{
    /// <summary>Who may use the tooling routes; see <see cref="ToolingAccess"/>.</summary>
    public ToolingAccess Tooling { get; set; } = ToolingAccess.LocalOnly;

    /// <summary>When set, it alone decides access to the tooling routes (unless they are disabled).</summary>
    public ToolingAuthorizer? ToolingAuthorizer { get; set; }

    /// <summary>
    /// Proxies (addresses or CIDR ranges) whose <c>X-Forwarded-For</c> / <c>X-Real-IP</c>
    /// name the client. Empty (the default): the TCP peer is the client, so no one
    /// can pick their own IP.
    /// </summary>
    public List<string> TrustedProxies { get; set; } = new();

    internal string ToolingName => Tooling switch
    {
        ToolingAccess.ApiKey => "api_key",
        ToolingAccess.Open => "open",
        ToolingAccess.Disabled => "disabled",
        _ => "local",
    };
}

/// <summary>
/// The core's access rules (octopus-core <c>tooling.rs</c> and <c>net.rs</c>), for the
/// .NET engine; on the core, the core itself decides.
/// </summary>
internal static class AccessRules
{
    public const string DenyNoApiKey = "Tooling requires an API key, but none is configured.";
    public const string DenyNotLocal = "The visualizer and execution history are only available from this machine. Configure an API key, a ToolingAuthorizer or EngineOptions.Tooling to allow remote access.";

    /// <summary>allow, not_found, ask_authorizer, require_api_key or deny, with the detail of a deny.</summary>
    public static (string Decision, string Detail) Decide(ToolingAccess tooling, bool hasAuthorizer, bool apiKeyConfigured, bool clientIsLoopback)
    {
        if (tooling == ToolingAccess.Disabled) return ("not_found", "");
        if (hasAuthorizer) return ("ask_authorizer", "");
        if (tooling == ToolingAccess.Open) return ("allow", "");
        if (tooling == ToolingAccess.ApiKey || apiKeyConfigured)
        {
            return apiKeyConfigured ? ("require_api_key", "") : ("deny", DenyNoApiKey);
        }
        return clientIsLoopback ? ("allow", "") : ("deny", DenyNotLocal);
    }

    /// <summary>The client's IP: the TCP peer, or the forwarded headers when the peer is a trusted proxy.</summary>
    public static string ClientIp(string? peer, string? forwardedFor, string? realIp, IEnumerable<string> trustedProxies)
    {
        peer = (peer ?? "").Trim();
        if (peer.Length == 0) return "127.0.0.1";
        if (!IPAddress.TryParse(peer, out var peerIp)) return peer;
        if (peerIp.IsIPv4MappedToIPv6) peerIp = peerIp.MapToIPv4();
        if (trustedProxies.Any(p => Contains(p, peerIp)))
        {
            var first = (forwardedFor ?? "").Split(',')[0].Trim();
            if (first.Length > 0) return first;
            var real = (realIp ?? "").Trim();
            if (real.Length > 0) return real;
        }
        return peerIp.ToString();
    }

    public static bool IsLoopback(string ip) =>
        ip == "localhost" || (IPAddress.TryParse(ip, out var a) && IPAddress.IsLoopback(a.IsIPv4MappedToIPv6 ? a.MapToIPv4() : a));

    private static bool Contains(string proxy, IPAddress ip)
    {
        proxy = proxy.Trim();
        if (!proxy.Contains('/'))
        {
            return IPAddress.TryParse(proxy, out var single)
                && (single.IsIPv4MappedToIPv6 ? single.MapToIPv4() : single).Equals(ip);
        }
        return IPNetwork.TryParse(proxy, out var net) && net.Contains(ip);
    }
}

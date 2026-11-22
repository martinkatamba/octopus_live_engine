using System.Reflection;
using System.Runtime.InteropServices;

namespace Octopus.LiveEngine.Core;

/// <summary>A buffer the core allocated; free it with <see cref="NativeMethods.octopus_buf_free"/>.</summary>
[StructLayout(LayoutKind.Sequential)]
internal unsafe struct OctopusBuf
{
    public byte* Ptr;
    public nuint Len;
    public nuint Cap;
}

/// <summary>
/// The core's C ABI (core/crates/octopus-ffi/include/octopus.h). Callbacks are
/// passed as unmanaged function pointers to <c>[UnmanagedCallersOnly]</c>
/// methods, so there are no delegates to keep alive.
/// </summary>
internal static unsafe partial class NativeMethods
{
    internal const string Library = "octopus_ffi";

    /// <summary>The contract this SDK was written against; see <c>CONTRACT_VERSION</c> in the core.</summary>
    internal const uint ExpectedContractVersion = 1;

    internal const int OK = 0;
    internal const int ErrArgument = 1;
    internal const int ErrNotFound = 2;
    internal const int ErrEngine = 3;
    internal const int ErrWorkflow = 4;
    internal const int ErrPanic = 5;
    internal const uint StateNoTracePayloads = 1;

    /// <summary>Overrides where the library is loaded from.</summary>
    internal const string LibraryEnvVar = "OCTOPUS_CORE_LIB";

    static NativeMethods()
    {
        NativeLibrary.SetDllImportResolver(typeof(NativeMethods).Assembly, Resolve);
    }

    private static string FileName =>
        OperatingSystem.IsWindows() ? "octopus_ffi.dll"
        : OperatingSystem.IsMacOS() ? "liboctopus_ffi.dylib"
        : "liboctopus_ffi.so";

    /// <summary>
    /// <c>OCTOPUS_CORE_LIB</c>, then the usual probing (a NuGet package's
    /// <c>runtimes/&lt;rid&gt;/native</c>), then a build in this repository's
    /// <c>core/target</c>, for development.
    /// </summary>
    private static IntPtr Resolve(string name, Assembly assembly, DllImportSearchPath? searchPath)
    {
        if (name != Library) return IntPtr.Zero;
        var fromEnv = Environment.GetEnvironmentVariable(LibraryEnvVar);
        if (!string.IsNullOrEmpty(fromEnv)) return NativeLibrary.Load(fromEnv);
        if (NativeLibrary.TryLoad(name, assembly, searchPath, out var handle)) return handle;
        // The newest build wins, so a fresh debug build isn't shadowed by an old release one.
        var builds = DevelopmentBuilds().Where(File.Exists).Distinct()
            .OrderByDescending(File.GetLastWriteTimeUtc);
        foreach (var candidate in builds)
        {
            if (NativeLibrary.TryLoad(candidate, out handle)) return handle;
        }
        return IntPtr.Zero;
    }

    private static IEnumerable<string> DevelopmentBuilds()
    {
        foreach (var start in new[] { AppContext.BaseDirectory, Directory.GetCurrentDirectory() })
        {
            for (var dir = new DirectoryInfo(start); dir != null; dir = dir.Parent)
            {
                foreach (var root in new[] { "core", Path.Combine("octopus_live_engine", "core") })
                {
                    foreach (var profile in new[] { "release", "debug" })
                    {
                        yield return Path.Combine(dir.FullName, root, "target", profile, FileName);
                    }
                }
            }
        }
    }

    [LibraryImport(Library)]
    internal static partial OctopusBuf octopus_core_version();

    [LibraryImport(Library)]
    internal static partial uint octopus_contract_version();

    [LibraryImport(Library)]
    internal static partial OctopusBuf octopus_last_error();

    [LibraryImport(Library)]
    internal static partial void octopus_buf_free(OctopusBuf buf);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial IntPtr octopus_engine_new(string? configJson);

    [LibraryImport(Library)]
    internal static partial void octopus_engine_free(IntPtr engine);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_register_blueprint(IntPtr engine, string blueprint);

    /// <summary><paramref name="call"/>: <c>int (void*, byte*, nuint, byte*, nuint, void*)</c>; <paramref name="has"/>: <c>bool (void*, byte*, nuint)</c>.</summary>
    [LibraryImport(Library)]
    internal static partial int octopus_set_activity_resolver(IntPtr engine, void* call, void* has, IntPtr userData);

    [LibraryImport(Library)]
    internal static partial void octopus_out_write(void* outBuf, byte* bytes, nuint len);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_execute_endpoint(
        IntPtr engine, string workflow, long endpointIndex, string? route, string? method,
        string? inputJson, uint flags, out OctopusBuf output);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_initiate_async(IntPtr engine, string workflow, string? inputJson, uint flags, out OctopusBuf output);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_save_state(IntPtr engine, string stateJson);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_get_state(IntPtr engine, string jobId, uint flags, out OctopusBuf output);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_list_states(IntPtr engine, string? workflow, nuint limit, uint flags, out OctopusBuf output);

    /// <summary><paramref name="callback"/>: <c>void (void*, byte*, nuint)</c>. Returns 0 on failure.</summary>
    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial ulong octopus_subscribe_events(IntPtr engine, string jobId, void* callback, IntPtr userData);

    [LibraryImport(Library)]
    internal static partial void octopus_unsubscribe(IntPtr engine, ulong subscription);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_blueprint_middleware_new(IntPtr engine, string workflow, nuint index, out IntPtr middleware);

    [LibraryImport(Library)]
    internal static partial void octopus_middleware_free(IntPtr middleware);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_middleware_run_before(IntPtr middleware, string requestJson, out OctopusBuf output);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_middleware_run_after(IntPtr middleware, string requestJson, string responseJson);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_client_ip(IntPtr engine, string? peer, string? forwardedFor, string? realIp, out OctopusBuf output);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_tooling_decision(IntPtr engine, string requestJson, out OctopusBuf output);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static partial bool octopus_api_key_allows(IntPtr engine, string? header, string? query);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_compile_endpoint_diagram(
        IntPtr engine, string workflow, string? jobId, string? route, string? method, out OctopusBuf output);

    [LibraryImport(Library, StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int octopus_visualizer_page(string routePrefix, string defaultDomain, string activityLanguage, out OctopusBuf output);

    [LibraryImport(Library)]
    internal static partial int octopus_start_worker(IntPtr engine);

    [LibraryImport(Library)]
    internal static partial int octopus_stop_worker(IntPtr engine);

    /// <summary>The buffer's bytes as UTF-8 text; frees the buffer.</summary>
    internal static string Take(OctopusBuf buf)
    {
        if (buf.Ptr == null) return string.Empty;
        try
        {
            return System.Text.Encoding.UTF8.GetString(buf.Ptr, checked((int)buf.Len));
        }
        finally
        {
            octopus_buf_free(buf);
        }
    }
}

using System.Collections.Concurrent;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;
using YamlDotNet.Core;
using YamlDotNet.Serialization;
using YamlDotNet.Serialization.NamingConventions;

namespace Octopus.LiveEngine.Adapters.Outbound;

public class ApiEndpointsYamlConverter : IYamlTypeConverter
{
    private static readonly Lazy<IDeserializer> InnerDeserializer = new(() =>
        new DeserializerBuilder()
            .WithNamingConvention(UnderscoredNamingConvention.Instance)
            .IgnoreUnmatchedProperties()
            .Build());

    public bool Accepts(Type type) => type == typeof(List<ApiEndpointConfig>);

    public object? ReadYaml(IParser parser, Type type)
    {
        if (parser.TryConsume<YamlDotNet.Core.Events.SequenceStart>(out _))
        {
            var list = new List<ApiEndpointConfig>();
            while (!parser.TryConsume<YamlDotNet.Core.Events.SequenceEnd>(out _))
            {
                var item = InnerDeserializer.Value.Deserialize<ApiEndpointConfig>(parser);
                if (item != null) list.Add(item);
            }
            return list;
        }

        if (parser.TryConsume<YamlDotNet.Core.Events.MappingStart>(out _))
        {
            var list = new List<ApiEndpointConfig>();
            while (!parser.TryConsume<YamlDotNet.Core.Events.MappingEnd>(out _))
            {
                var key = parser.Consume<YamlDotNet.Core.Events.Scalar>().Value;
                if (key == "endpoints" || key == "routes")
                {
                    if (parser.TryConsume<YamlDotNet.Core.Events.SequenceStart>(out _))
                    {
                        while (!parser.TryConsume<YamlDotNet.Core.Events.SequenceEnd>(out _))
                        {
                            var item = InnerDeserializer.Value.Deserialize<ApiEndpointConfig>(parser);
                            if (item != null) list.Add(item);
                        }
                    }
                }
                else
                {
                    parser.SkipThisAndNestedEvents();
                }
            }
            return list;
        }

        return new List<ApiEndpointConfig>();
    }

    public void WriteYaml(IEmitter emitter, object? value, Type type)
    {
    }
}

public class YamlBlueprintRepository : IBlueprintRepository
{
    private readonly ConcurrentDictionary<string, WorkflowBlueprint> _blueprints = new();
    private readonly IDeserializer _deserializer;

    public YamlBlueprintRepository()
    {
        _deserializer = new DeserializerBuilder()
            .WithNamingConvention(UnderscoredNamingConvention.Instance)
            .WithTypeConverter(new ApiEndpointsYamlConverter())
            .IgnoreUnmatchedProperties()
            .Build();
    }

    public Task SaveBlueprintAsync(WorkflowBlueprint blueprint, CancellationToken cancellationToken = default)
    {
        if (blueprint != null)
        {
            blueprint.NormalizeEndpoints();
            if ((blueprint.Steps == null || blueprint.Steps.Count == 0) && blueprint.Api != null && blueprint.Api.Count > 0 && blueprint.Api[0].Steps != null && blueprint.Api[0].Steps.Count > 0)
            {
                blueprint.Steps = blueprint.Api[0].Steps;
            }
            _blueprints[blueprint.WorkflowName] = blueprint;
        }
        return Task.CompletedTask;
    }

    public Task<WorkflowBlueprint?> GetBlueprintAsync(string workflowName, CancellationToken cancellationToken = default)
    {
        _blueprints.TryGetValue(workflowName, out var bp);
        return Task.FromResult(bp);
    }

    public Task<IReadOnlyList<WorkflowBlueprint>> ListBlueprintsAsync(CancellationToken cancellationToken = default)
    {
        IReadOnlyList<WorkflowBlueprint> list = _blueprints.Values.ToList();
        return Task.FromResult(list);
    }

    public void LoadFromDirectory(string directoryPath)
    {
        if (!Directory.Exists(directoryPath)) return;

        var files = Directory.GetFiles(directoryPath, "*.yaml")
            .Concat(Directory.GetFiles(directoryPath, "*.yml"));

        foreach (var file in files)
        {
            try
            {
                var yaml = File.ReadAllText(file);
                var bp = _deserializer.Deserialize<WorkflowBlueprint>(yaml);
                if (bp != null && !string.IsNullOrWhiteSpace(bp.WorkflowName))
                {
                    if ((bp.Steps == null || bp.Steps.Count == 0) && bp.Api != null && bp.Api.Count > 0 && bp.Api[0].Steps != null && bp.Api[0].Steps.Count > 0)
                    {
                        bp.Steps = bp.Api[0].Steps;
                    }
                    _blueprints[bp.WorkflowName] = bp;
                }
            }
            catch { /* Ignore unparseable files */ }
        }
    }

    public void LoadFromFile(string filePath)
    {
        var resolvedPath = filePath;
        if (!File.Exists(resolvedPath))
        {
            var baseCombined = Path.Combine(AppContext.BaseDirectory, filePath);
            if (File.Exists(baseCombined)) resolvedPath = baseCombined;
        }

        if (!File.Exists(resolvedPath))
        {
            var cur = new DirectoryInfo(AppContext.BaseDirectory);
            while (cur != null)
            {
                var candidate = Path.Combine(cur.FullName, filePath);
                if (File.Exists(candidate))
                {
                    resolvedPath = candidate;
                    break;
                }
                cur = cur.Parent;
            }
        }

        if (!File.Exists(resolvedPath))
        {
            var cur = new DirectoryInfo(Directory.GetCurrentDirectory());
            while (cur != null)
            {
                var candidate = Path.Combine(cur.FullName, filePath);
                if (File.Exists(candidate))
                {
                    resolvedPath = candidate;
                    break;
                }
                cur = cur.Parent;
            }
        }

        if (!File.Exists(resolvedPath)) return;

        var yaml = File.ReadAllText(resolvedPath);
        var bp = _deserializer.Deserialize<WorkflowBlueprint>(yaml);
        if (bp != null && !string.IsNullOrWhiteSpace(bp.WorkflowName))
        {
            if ((bp.Steps == null || bp.Steps.Count == 0) && bp.Api != null && bp.Api.Count > 0 && bp.Api[0].Steps != null && bp.Api[0].Steps.Count > 0)
            {
                bp.Steps = bp.Api[0].Steps;
            }
            _blueprints[bp.WorkflowName] = bp;
        }
    }
}



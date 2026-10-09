using Octopus.LiveEngine.Domain.Ports;

namespace Octopus.LiveEngine.Domain.Services;

public record SagaAction(string StepId, string Compensation, Dictionary<string, object> Payload);

public class SagaManager
{
    private readonly IActivityExecutor _activityExecutor;

    public SagaManager(IActivityExecutor activityExecutor)
    {
        _activityExecutor = activityExecutor;
    }

    public async Task<List<Exception>> ExecuteCompensationsAsync(
        IReadOnlyList<SagaAction> actions,
        CancellationToken cancellationToken = default
    )
    {
        var exceptions = new List<Exception>();
        for (int i = actions.Count - 1; i >= 0; i--)
        {
            var action = actions[i];
            if (string.IsNullOrWhiteSpace(action.Compensation)) continue;

            if (_activityExecutor.HasActivity(action.Compensation))
            {
                try
                {
                    await _activityExecutor.ExecuteActivityAsync(action.Compensation, action.Payload, cancellationToken);
                }
                catch (Exception ex)
                {
                    exceptions.Add(new InvalidOperationException($"Compensation '{action.Compensation}' failed for step '{action.StepId}'", ex));
                }
            }
        }
        return exceptions;
    }
}

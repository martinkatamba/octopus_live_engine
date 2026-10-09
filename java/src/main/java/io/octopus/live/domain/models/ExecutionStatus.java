package io.octopus.live.domain.models;

public enum ExecutionStatus {
    IDLE,
    RUNNING,
    COMPLETED,
    FAILED,
    ROLLED_BACK,
    CANCELLED
}

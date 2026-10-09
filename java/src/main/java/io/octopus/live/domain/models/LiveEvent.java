package io.octopus.live.domain.models;

public class LiveEvent {
    private String event;
    private Object data;

    public LiveEvent() {}

    public LiveEvent(String event, Object data) {
        this.event = event;
        this.data = data;
    }

    public String getEvent() { return event; }
    public void setEvent(String event) { this.event = event; }
    public Object getData() { return data; }
    public void setData(Object data) { this.data = data; }
}

package io.octopus.live.domain.services.middleware;

public class ResponseAuditMiddleware extends LiveMiddleware {

    public ResponseAuditMiddleware() {
        super("ResponseAuditMiddleware");
        this.type = "post-request";
        this.activity = "RecordResponseAuditLog";
        this.async = true;
    }

    public ResponseAuditMiddleware(String activity, boolean async) {
        super("ResponseAuditMiddleware");
        this.type = "post-request";
        this.activity = activity != null ? activity : "RecordResponseAuditLog";
        this.async = async;
    }

    @Override
    public void process(LiveRequestContext ctx) {
        // Post-request middleware: pre-request phase is a no-op
    }
}

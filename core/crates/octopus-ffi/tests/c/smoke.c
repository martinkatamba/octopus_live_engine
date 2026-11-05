/* Uses the C ABI from C: a C activity, a run, a failure and the last error. */
#include <stdio.h>
#include <string.h>
#include "octopus.h"

static const char *BLUEPRINT =
    "workflow_name: greet\n"
    "version: \"1.0.0\"\n"
    "steps:\n"
    "  - id: hello\n"
    "    activity: Hello\n"
    "    is_output: true\n";

static int32_t hello(void *user_data, const uint8_t *input, size_t len, OctopusOut *out) {
    int *calls = user_data;
    (*calls)++;
    const char *result = memmem(input, len, "nobody", 6) ? "no one to greet" : "{\"greeting\": \"hi\"}";
    octopus_out_write(out, (const uint8_t *)result, strlen(result));
    return memmem(input, len, "nobody", 6) ? 1 : OCTOPUS_OK;
}

static int contains(OctopusBuf buf, const char *needle) {
    int found = memmem(buf.ptr, buf.len, needle, strlen(needle)) != NULL;
    octopus_buf_free(buf);
    return found;
}

#define CHECK(cond) do { if (!(cond)) { fprintf(stderr, "failed: %s (line %d)\n", #cond, __LINE__); return 1; } } while (0)

int main(void) {
    CHECK(octopus_contract_version() == 0);
    OctopusEngine *engine = octopus_engine_new(NULL);
    CHECK(engine != NULL);
    int calls = 0;
    CHECK(octopus_register_blueprint(engine, BLUEPRINT) == OCTOPUS_OK);
    CHECK(octopus_register_activity(engine, "Hello", hello, &calls) == OCTOPUS_OK);

    OctopusBuf state;
    CHECK(octopus_execute(engine, "greet", "{\"name\": \"Ada\"}", &state) == OCTOPUS_OK);
    CHECK(contains(state, "\"COMPLETED\""));
    CHECK(calls == 1);

    CHECK(octopus_execute(engine, "greet", "{\"name\": \"nobody\"}", &state) == OCTOPUS_ERR_WORKFLOW);
    CHECK(contains(state, "\"FAILED\""));
    CHECK(contains(octopus_last_error(), "no one to greet"));
    CHECK(octopus_execute(engine, "greet", "{oops", &state) == OCTOPUS_ERR_ARGUMENT);

    octopus_engine_free(engine);
    printf("C smoke test passed\n");
    return 0;
}

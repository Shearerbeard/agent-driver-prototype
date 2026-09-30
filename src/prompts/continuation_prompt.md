ITERATION %%ITERATION%% of %%MAX_ITERATIONS%%%%URGENCY%%

Goal (verbatim from the original request): %%GOAL%%
Outcome: %%SUCCEEDED%% of %%TOTAL%% tasks succeeded.

%%COMPLETED_SECTION%%%%BLOCKED_SECTION%%%%REDESIGN_SECTION%%%%FAILURE_SECTION%%%%FAILURE_HISTORY%%%%REUSE_GUIDANCE%%
This is an end-of-iteration decision point. Choose the next call:

- `respond` — write the final answer from the results above, plus the tools available to you and general knowledge.
- `create_plan` — issue a new plan when the current results point to the next step: a deeper investigation into what they revealed (e.g. narrowing from identified failure groups into their affected apps), a step they expose as missing, or retrying failed tasks with a different approach.
- `inspect_run` — read back a task's full evidence when its inline preview is truncated or insufficient for your decision.

When you can answer the user from what's already available to you, call `respond`. When more worker tool work is needed, call `create_plan` — then `execute` the new plan once it is recorded.

IMPORTANT — synthesis rules for `respond`:
Your response IS the final answer the user sees. Task results are NOT shown to the user. You must inline all relevant findings — exact names, values, identifiers, and data points from the task results above. Never reference tasks by number or defer to task outputs. Extract the concrete data and present it directly.

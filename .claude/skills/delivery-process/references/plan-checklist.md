# Plan checklist (planning only)

Used by whoever writes or reviews a plan (tech lead, architect, a domain reviewer asked to look at the plan), never as
a developer's checklist. Each case that applies to this feature is written into the plan's **acceptance criteria** as
a concrete Given / When / Then, so the developer builds from requirements and the final reviewers check against the
same criteria. A case that doesn't apply is simply left out; nothing from this list is copied into a slice brief.

## Failure paths
- A write or call fails halfway: what the user sees, what is rolled back or recorded.
- Missing, unknown or zero values (config, data, a threshold): the feature refuses or warns; it never passes as fine.
- Retries, queues and timeouts are bounded; a timed-out request can't take effect later unnoticed.

## Controls that must be disabled
For every control that must not be used in some state (pay, submit, approve, place order, retry), write one criterion
per way it could still be enabled, for example "Given the network is offline, when a payment is pending, then Pay stays
disabled":
- a timer that fires late or early;
- the clock stepping forward or back;
- a sleeping or throttled tab;
- an offline or paused request;
- a refetch that failed or is still running;
- the state checked again at click time, not only at render;
- a queued mutation counting as in flight.

## Safety rules
- Each refusal or limit is named, so a test can prove which rule refused.

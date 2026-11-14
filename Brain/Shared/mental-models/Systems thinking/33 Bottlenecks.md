# Bottlenecks

## Description

**Bottlenecks** is a mental model drawn from Eliyahu Goldratt's **Theory of Constraints** (1984): every system has exactly one binding constraint at any given time — the bottleneck — that limits the total throughput of the entire system. No matter how much you optimize other parts, total output cannot exceed what the bottleneck allows.

- **Origins:** Goldratt introduced the Theory of Constraints in his novel *The Goal* (1984), which remains one of the most influential business books ever written. The core insight — that a chain is only as strong as its weakest link — is ancient, but Goldratt formalized it into a rigorous management methodology.

- **Key Principles:**
    - **The constraint limits everything:** Improving non-bottleneck processes is waste if the bottleneck isn't addressed. An hour lost at the bottleneck is an hour lost by the entire system. An hour saved at a non-bottleneck is a mirage.
    - **Bottlenecks shift:** Once you fix the current bottleneck, a new one immediately emerges. Improvement is a continuous cycle of finding and elevating constraints.
    - **Inventory before the bottleneck is waste:** Work-in-progress piling up before a bottleneck is a visible symptom of the constraint.
    - **Exploit before invest:** Before adding capacity to a bottleneck, ensure you are fully utilizing its current capacity — idle time at the bottleneck is the most expensive waste.
    - **The 5 Focusing Steps:** (1) Identify the constraint, (2) Exploit the constraint, (3) Subordinate everything else to the constraint, (4) Elevate the constraint, (5) Return to step 1.

- **Real-World Examples:** In software development, a single slow code review process can stall an entire engineering team regardless of how fast developers write code. In a factory, one slow machine determines the factory's output rate. In personal productivity, your most mentally demanding task is your cognitive bottleneck — schedule it at peak energy.

---

## When to Use

- **Diagnosing why output is lower than expected:** Identify where work accumulates and trace back to the constraint.
- **Prioritizing improvement investments:** Focus exclusively on bottleneck improvement to maximize system throughput.
- **Process design:** Design pipelines so the bottleneck has maximum utilization and non-bottleneck stages pace themselves to it.
- **Personal productivity:** Identify the one constraint on your most important goal and subordinate other activities to removing it.
- **Project management:** Identify the critical path — the sequence of dependent tasks that determines project duration.

---

## When to Avoid

- **Assuming one bottleneck explains everything:** Highly variable systems (e.g., creative work, complex negotiations) may have shifting or multiple simultaneous constraints.
- **Over-focusing at the expense of systemic health:** Obsessively exploiting one bottleneck can starve other parts of the system that need maintenance and investment.
- **In truly parallel, independent systems:** When components don't interact in a pipeline, bottleneck theory doesn't apply.

---

## Thinking Steps

1. **Map the system as a pipeline:** Identify the sequence of stages through which work, material, or value flows.
2. **Measure throughput at each stage:** Where does work accumulate? Where is utilization highest?
3. **Identify the bottleneck:** The stage with the highest utilization and the most queue before it is the constraint.
4. **Exploit the constraint:** Maximize the bottleneck's output — remove idle time, defects, and interruptions at that single stage.
5. **Subordinate everything else:** Other stages should pace themselves to feed and not starve the bottleneck. Excess speed elsewhere just creates inventory.
6. **Elevate the constraint:** If exploitation isn't enough, add capacity — more people, machines, or time at the bottleneck only.
7. **Reassess:** Once the bottleneck is resolved, find the next one. Resist the temptation to declare victory.
8. **Beware of policies as constraints:** Often the binding constraint is a policy (approval process, meeting cadence) not a physical limitation.

# Optimization

## Description

**Optimization** is the process of finding the best possible solution from a set of alternatives, subject to constraints. It is both a mathematical discipline and a cognitive framework for systematic decision-making. But optimization also has deep limits: real-world systems have multiple objectives, noisy data, shifting constraints, and unintended consequences — making over-optimization a failure mode as dangerous as under-optimization.

- **Origins:** Mathematical optimization dates to the calculus of Leibniz and Newton. Linear programming was developed in WWII by George Dantzig. As a general mental model, its limits were illuminated by Goodhart's Law (when a measure becomes a target, it ceases to be a good measure) and by complexity scientists who showed that over-optimized systems become brittle.

- **Key Principles:**
    - **Objective function:** Optimization requires a single, clearly defined goal to maximize or minimize. When there are multiple objectives, you must either combine them (with weights) or accept trade-offs.
    - **Constraints:** Optimization always occurs within constraints. Changing the constraints changes the optimal solution.
    - **Local vs. global optima:** Optimization algorithms often find local optima — the best nearby solution — not the global optimum. See: Global and Local Maxima.
    - **Goodhart's Law:** When you optimize hard for a proxy metric, behavior shifts to game the metric rather than achieve the underlying goal. The metric becomes useless as a target.
    - **Robustness vs. optimality:** A system optimized for one scenario may perform catastrophically in another. Robust solutions sacrifice optimality in any single scenario for better performance across many scenarios.
    - **The efficiency-resilience trade-off:** Highly optimized systems eliminate redundancy (which looks like waste) but become fragile — just-in-time supply chains are optimal in normal conditions, disastrous in disruptions.

- **Real-World Examples:** Amazon's logistics optimization reduces delivery costs but creates single points of failure during disruptions. A hospital that optimizes for throughput may sacrifice patient safety in edge cases. An athlete who optimizes every variable (sleep, nutrition, training) may become brittle to any deviation. A company that optimizes for quarterly earnings destroys long-term value.

---

## When to Use

- **Resource allocation with clear objectives:** When the objective is well-defined and the constraints are known, systematic optimization beats intuitive allocation.
- **Process improvement:** Use optimization thinking to eliminate waste, reduce cycle times, and improve quality in well-understood processes.
- **Algorithm design:** In software and data systems, formal optimization methods (linear programming, gradient descent) produce provably better solutions than heuristics.
- **Decision frameworks:** Pre-defining the objective function forces clarity about what you actually want before making decisions.

---

## When to Avoid

- **Multiple conflicting objectives:** Over-reducing complex, multi-objective situations to a single metric produces Goodhart's Law failures.
- **High-uncertainty environments:** Optimizing for a specific predicted future that may not materialize is costly. Favor robustness.
- **When the model is wrong:** Optimizing based on a flawed model produces solutions that are optimal within the model but terrible in reality.
- **Human systems with gaming behavior:** Whenever people know the optimization criteria, they will optimize for the metric rather than the underlying goal.

---

## Thinking Steps

1. **Define the objective function precisely:** What exactly are you trying to maximize or minimize?
2. **List the constraints:** What cannot change? What are the limits on your variables?
3. **Identify all variables:** What can you actually control?
4. **Check for multiple objectives:** Are there secondary goals? How will you trade them off?
5. **Search the solution space:** Systematically explore alternatives — don't just improve incrementally from the starting point.
6. **Test for Goodhart's Law:** Is the metric you're optimizing a genuine measure of the goal, or a proxy that can be gamed?
7. **Assess robustness:** How does the optimal solution perform in scenarios other than the one you optimized for?
8. **Implement, measure, and reoptimize:** Once implemented, track whether the objective is actually being achieved and recalibrate.

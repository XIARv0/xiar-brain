# Regression to the Mean

## Description

**Regression to the Mean** is the statistical phenomenon where extreme measurements — unusually high or low values — tend to be followed by values closer to the average. This occurs not because of any intervention or causal force, but because extreme values contain a large random component that is unlikely to repeat. It is one of the most misunderstood and consequential statistical concepts in everyday life.

- **Origins:** Discovered by Sir Francis Galton in the 1880s while studying the heights of children relative to their parents. He found that very tall parents tended to have children who were tall but less extreme — "regressing" toward the population mean. Galton initially called it "reversion to mediocrity." Daniel Kahneman later identified regression to the mean as the source of a pervasive cognitive illusion — people falsely attribute regression to their own interventions.

- **Key Principles:**
    - **Extreme values are extreme partly by luck:** Any measurement combines true underlying level + random noise. High measurements often reflect both a high true level AND a lucky random component. The luck is unlikely to repeat.
    - **The intervention illusion:** When we intervene after an extreme observation and the next measurement is closer to average, we attribute the improvement to our intervention. But the regression would have occurred without intervention.
    - **Kahneman's flight instructor example:** Instructors praised pilots after exceptional landings and criticized them after poor ones. The praised pilots did worse next time; the criticized pilots did better. The instructors concluded that praise harms and criticism helps — but they were observing regression, not causation.
    - **Bidirectional:** Regression works both ways — very poor performance tends to improve, and very strong performance tends to decline, on the next measurement.
    - **No memory in the noise:** The random component of a measurement has no memory — each measurement's noise is independent.

- **Real-World Examples:** Sports rookies of the year often underperform the following season (the "sophomore slump"). Companies with exceptional first-quarter earnings often have a more modest second quarter. Countries with unusually high economic growth one year tend to grow more slowly the next. Medical patients who seek treatment when symptoms are at their worst tend to improve — regardless of treatment efficacy.

---

## When to Use

- **Evaluating interventions:** Before claiming an action caused an improvement, ask whether regression to the mean could explain the result.
- **Performance management:** Avoid concluding that punishment works and praise doesn't based on one observation after each.
- **Designing clinical trials:** Use control groups specifically to separate regression to the mean from genuine treatment effects.
- **Forecasting:** After observing extreme performance, forecast toward the mean rather than extrapolating the extreme.
- **Hiring and sports:** Recognize that exceptional performance in a single interview, audition, or game may not reflect stable underlying ability.

---

## When to Avoid

- **When dealing with true trends, not noise:** If a system has a genuine structural change (a new CEO, a new technology), regression to the mean doesn't necessarily apply — the underlying mean has shifted.
- **When the random component is small:** In low-noise, high-skill domains, extreme performance is more likely to reflect genuine ability and persist.

---

## Thinking Steps

1. **Identify the extreme observation:** What unusually high or low measurement prompted attention or action?
2. **Decompose signal from noise:** How much of the extreme value is likely due to underlying true level vs. random fluctuation?
3. **Ask the counterfactual:** Would the next measurement likely regress toward the mean regardless of any intervention?
4. **Before intervening, set a baseline:** What does the next measurement look like without intervention? Compare to the intervened condition.
5. **Use control groups:** In any evaluation of an intervention, compare to a group that experienced the same extreme condition without the intervention.
6. **Forecast toward the mean:** When making predictions following extreme observations, weight toward average performance rather than extrapolating the extreme.
7. **Document your prediction:** Before the next measurement, explicitly predict regression — then compare to what actually happens to calibrate your intuition.

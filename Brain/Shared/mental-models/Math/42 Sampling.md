# Sampling

## Description

**Sampling** is the process of drawing inferences about a large population by observing a smaller subset. Because we can rarely examine every instance of anything — every customer, every outcome, every data point — we must rely on samples. The quality of our conclusions depends critically on whether our sample is representative and large enough.

- **Origins:** Statistical sampling theory was formalized by mathematicians including Karl Pearson and Ronald Fisher in the early 20th century, enabling rigorous inference from partial data. In everyday thinking, the mental model was popularized by Daniel Kahneman and Amos Tversky, who documented how humans routinely violate good sampling principles — judging populations from tiny, biased samples.

- **Key Principles:**
    - **Representativeness:** A sample must reflect the distribution of the population to produce valid inferences. A non-representative sample produces systematically wrong conclusions regardless of size.
    - **Sample size and confidence:** Larger samples produce more reliable estimates — but with diminishing returns. Doubling precision requires quadrupling sample size (due to the square root relationship in standard error).
    - **Sampling bias:** The most dangerous errors come from systematic bias in how the sample was drawn — not from random noise. Biased samples can be large and still produce wrong answers.
    - **Your personal experience is a sample:** Your life experience, your social circle, your news feed — all are tiny, heavily biased samples of the full distribution of human experience. Generalizing from them is statistically unsound.
    - **Survivorship bias as sampling error:** Studying only successful companies, surviving veterans, or published studies is sampling from a biased subset — the failures are excluded.

- **Real-World Examples:** The 1936 Literary Digest poll predicted a Landon landslide over Roosevelt because its sample was drawn from telephone and car owners — the wealthy. It was large (2.4M responses) but catastrophically biased. Abraham Wald's analysis of WWII aircraft armor required explicitly accounting for the fact that only returning planes were sampled.

---

## When to Use

- **Evaluating research and evidence:** Always ask: how was the sample drawn? Is it representative? How large is it?
- **Business decisions:** Before generalizing from customer feedback, A/B tests, or user interviews, assess sampling quality.
- **Personal belief auditing:** Recognize that your strongly held beliefs about "how people are" or "what works" are based on limited personal samples.
- **Data analysis:** Before drawing conclusions from data, examine how the data was collected and what biases the collection method introduces.

---

## When to Avoid

- **When you have the full population:** If you have all the data (a census), sampling theory is unnecessary — analyze it directly.
- **When sample size is truly adequate:** Don't over-caveat well-designed large samples. Perfect is the enemy of useful.

---

## Thinking Steps

1. **Define the population you want to understand:** What is the full set of entities, events, or outcomes you care about?
2. **Examine how the sample was drawn:** Was it random? Systematic? Self-selected? Each method has different bias risks.
3. **Assess representativeness:** Does the sample reflect the key dimensions of variation in the population?
4. **Check for exclusions:** Who or what is missing from the sample? What systematic forces determine who is included?
5. **Evaluate size relative to variability:** Is the sample large enough to detect the effect you care about, given the population's variance?
6. **Quantify uncertainty:** What is the confidence interval around your estimate? Express conclusions with appropriate uncertainty.
7. **Identify the strongest bias risk:** What is the most likely way this sample misleads you? Weight your conclusions accordingly.
8. **Seek disconfirming samples:** Actively look for populations or data sources that might contradict your sample-derived conclusions.

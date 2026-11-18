# Evidence behind the focus features

This note records why a few StayProductive behaviours are shaped the way they
are. It is a design constraint, not a claim that the app has been shown to
change anyone's habits. The studies below are about plans, monitoring,
commitment, breaks, and habits in general. None of them evaluated this program.

## What the product should do

1. Ask for a specific if-then plan, and show it again later.
2. Report time against the period the person chose, without grading the person.
3. Make a pause easy to start and obvious about when it ends.
4. Treat a strict session as a voluntary commitment with a visible cool-down
   and a working way out. It is not a lock the person cannot leave.
5. Suggest a break as recovery, without a supposed optimal length.

## Implementation intentions

A goal intention ("I want to focus") is weaker than a plan that names the
situation and the response: "If I open a chat app during work, then I will
leave it until this window ends."

Gollwitzer (1999) described implementation intentions as if-then plans that
link a cue to a response. Gollwitzer and Sheeran (2006) reviewed 94 tests and
reported a medium-to-large benefit for goal attainment, d = 0.65 (95% CI 0.60
to 0.70). The benefit showed up for getting started, shielding a pursuit from
distraction, and letting go of a failing approach. It is not a guarantee, and
the plans in those studies were written for a specific behaviour, not a vague
wish.

Product use: onboarding can store an optional if-then plan on the first
profile. The dashboard and the profile card repeat the sentences the person
wrote. The engine does not treat the plan as a rule.

## Self-monitoring and feedback

Harkin and colleagues (2016) meta-analysed 138 randomized studies of monitoring
progress toward a goal. Prompts to monitor raised attainment (d+ = 0.40, 95%
CI 0.32 to 0.48). The gain was larger when progress was written down, and when
it was reported to someone else, mostly for health goals. Monitoring a
behaviour moved that behaviour. The practical reading for this app is a local
today / week / month record of the apps the person chose to track. Publishing
that record is not required.

Kluger and DeNisi (1996) found that feedback helped on average (d = 0.41) and
that more than a third of interventions reduced performance. Benefit shrank as
attention moved from the task toward the self. Praise, discouragement, and
comparison with other people pull attention that way. Stats in this app
therefore say where time went and how that compares with the previous period
of the same length. They do not score the person.

## Friction and commitment

Bryan, Karlan, and Nelson (2010) review commitment devices: arrangements people
set up now to make a later choice harder or more expensive. Rogers, Milkman,
and Volpp (2014) describe the ones that have been tried as voluntary, including
soft devices a person can later leave. A device that cannot be left at all is
a different product, and it is outside what this desktop app can honestly
promise. The same local user can read the state file. Strict mode here is a
cool-down and a refusal to edit rules while the commitment is active, with the
end time on screen.

Duckworth, Gendler, and Gross (2016) group self-control strategies and put
situation selection and situation modification ahead of relying on in-the-moment
willpower. Blocking an app the person already named is that kind of change to
the situation. It is not evidence that blocking every possible workaround
works, or that a harder block is always better.

## Breaks

Hunter and Wu (2016) followed 95 employees for five days and found that breaks
people preferred, and breaks earlier in the day, were associated with more
recovery. Longer breaks and frequent short breaks both beat rare short ones.
The study is observational and does not nominate a minute count. Trougakos,
Beal, Green, and Weiss (2008) found that how people spent a long break, not
only that they stopped, was tied to how they felt afterward. Claims such as a
universal 52/17 split, a 25-minute Pomodoro as a proven optimum, or an
ultradian rhythm that the app should enforce are not used here.

Product use: a pause lists a few durations, the rest of today, and "until I
resume," and the screen says when it ends. The person picks the length.

## Habits

Lally, van Jaarsveld, Potts, and Wardle (2010) followed 96 volunteers repeating
one chosen daily behaviour in the same context for 12 weeks. For the 39 people
whose automaticity fit a curve, the median time to 95% of their own plateau
was 66 days, with a range of 18 to 254. Estimates past day 84 are
extrapolations. One missed day did not materially change the curve. That
median is not a target, and the sample was not about screen time. Wood and
Neal (2007) describe habits as responses tied to stable contexts, not to
repetition alone. A profile that runs on the same days and hours is a context
cue. The app does not tell anyone they have formed a habit after a set number
of days.

## What the interface must not say

- That the person is addicted, failing, or weak.
- That a particular break length is scientifically optimal.
- That 21 or 66 days of use will create a habit.
- That a strict session cannot be ended. The cool-down is the end path, and
  the state stays on the person's machine.
- That these citations are a clinical claim about this app.

## Sources

- Bryan, G., Karlan, D., & Nelson, S. (2010). Commitment devices. *Annual
  Review of Economics, 2*, 671–698. https://doi.org/10.1146/annurev.economics.102308.124324
- Duckworth, A. L., Gendler, T. S., & Gross, J. J. (2016). Situational
  strategies for self-control. *Perspectives on Psychological Science, 11*(1),
  35–55. https://doi.org/10.1177/1745691615623247
- Gollwitzer, P. M. (1999). Implementation intentions: Strong effects of simple
  plans. *American Psychologist, 54*(7), 493–503.
  https://doi.org/10.1037/0003-066X.54.7.493
- Gollwitzer, P. M., & Sheeran, P. (2006). Implementation intentions and goal
  achievement: A meta-analysis of effects and processes. *Advances in
  Experimental Social Psychology, 38*, 69–119.
  https://doi.org/10.1016/S0065-2601(06)38002-1
- Harkin, B., Webb, T. L., Chang, B. P. I., Prestwich, A., Conner, M., Kellar,
  I., Benn, Y., & Sheeran, P. (2016). Does monitoring goal progress promote
  goal attainment? A meta-analysis of the experimental evidence.
  *Psychological Bulletin, 142*(2), 198–229. https://doi.org/10.1037/bul0000025
- Hunter, E. M., & Wu, C. (2016). Give me a better break: Choosing workday
  break activities to maximize resource recovery. *Journal of Applied
  Psychology, 101*(2), 302–311. https://doi.org/10.1037/apl0000045
- Kluger, A. N., & DeNisi, A. (1996). The effects of feedback interventions on
  performance: A historical review, a meta-analysis, and a preliminary
  feedback intervention theory. *Psychological Bulletin, 119*(2), 254–284.
  https://doi.org/10.1037/0033-2909.119.2.254
- Lally, P., van Jaarsveld, C. H. M., Potts, H. W. W., & Wardle, J. (2010). How
  are habits formed: Modelling habit formation in the real world. *European
  Journal of Social Psychology, 40*(6), 998–1009. https://doi.org/10.1002/ejsp.674
- Rogers, T., Milkman, K. L., & Volpp, K. G. (2014). Commitment devices: Using
  initiatives to change behavior. *JAMA, 311*(20), 2065–2066.
  https://doi.org/10.1001/jama.2014.3485
- Trougakos, J. P., Beal, D. J., Green, S. G., & Weiss, H. M. (2008). Making the
  break count: An episodic examination of recovery activities, emotional
  experiences, and positive affective displays. *Academy of Management Journal,
  51*(1), 131–146. https://doi.org/10.5465/amj.2008.30764063
- Wood, W., & Neal, D. T. (2007). A new look at habits and the habit–goal
  interface. *Psychological Review, 114*(4), 843–863.
  https://doi.org/10.1037/0033-295X.114.4.843

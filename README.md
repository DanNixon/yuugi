# yuugi

A quick, hacky proof of concept tool for monitoring power consumption of multi-component software systems.

See `yuugi --help` for details of configurable options.

Currently only provides a vague estimate of power consumption due to horrific assumptions about the state of the system's CPU but is better than nothing and using process CPU time as a proxy measurement is still valid to identifiy possible optimisation targets.

The following caveats apply to using this:

- CPU power must be manually derived (if in doubt the TDP will be a sensible guess, this will in most cases result in an overestimation of power consumption)
- CPU frequency/power states are not taken into account (if a core is halted or running at a lower frequency then power consumption will be overestimated)
- Mutliple socket systems will probably not "just work"
- Short lived processes will not be reported correctly if their life is less than or not significantly longer than the polling interval
- The current label set for CPU time and energy generates a *lot* of series (be sure whatever collection system to feed this into is suitable to do so)

TL;DR: probably just use the CPU time measurement.

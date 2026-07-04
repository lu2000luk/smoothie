# smoothie/router

Implement:
/allow?domain={} -> 200 (true) / 500 (false)
/route?host={}&scheme={}&id={}&ip={} -> [{"dial":"host:port"}] (choose port, connect tunnel if necessary, prepare hypervisor)

With logging and caching.

TODO:
- Communication with hypervisor
- Communication with tunnel
- Run container on hosts that have already ran it so the state is cached (needs some work to be done)

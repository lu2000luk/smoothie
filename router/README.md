# Smoothie/router

Implement:
/allow?domain={} -> 200 (true) / 500 (false)
/route?host={}&scheme={}&id={}&ip={} -> [{"dial":"host:port"}] (choose port, connect tunnel if necessary, prepare hypervisor)

With logging and caching.

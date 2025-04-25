# frr-agent

* A daemon to reload FRR configurations.
* The daemon listens on a unix stream socket expecting configs and calls frr-reload to apply them.
* All parameters come from the cmd line. The only mandatory parameter is the address to bind the unix socket to.
* The daemon is purposedly designed to handle a single connection at a time and to disconnect if ever the decoding
  of a message fails, immediately transitioning to accepting a new connection.
* The frr-agent expects data to be minimally serialized as follows.
  Every message (sent or received) has the following structure on the wire:
```
      length(8 octets)|genid(8 octets)|message(length octets)|
```
  with:
  
  * length = size of the message in octets, encoded in 8 octets (host endianness)
  * genid = generation id of the message (e.g. a config or response). In keepalives it is expected to be zero.
  * message = the actual message as a string, which can be
      "KEEPALIVE" in keepalives or a config BLOB in requests (incoming messages)
      "Ok" or a blob including a failure (outgoing messages)

# cmd line args

The complete set of cmd line args is the following:
```
Daemon to reload FRR configs

Usage: frr-agent [OPTIONS] --sock-path <Unix socket bind path>

Options:
      --sock-path <Unix socket bind path>
      --loglevel <Loglevel (error, warn, info, debug, trace). Defaults to debug>
      --outdir <Directory where received configs are stored>
      --reloader <Full path to reloader (frr-reload.bin|py)>
      --bindir <Directory of vtysh>
      --rundir <Directory of where frr-reload writes temp files>
      --confdir <Directory of frr config files>
      --vtysock <vtysh sock (UNUSED atm)>
      --always-ok
      --proc-time <Artificially increase processing time by this number of seconds>
  -h, --help                                                                         Print help
  -V, --version
```

* confdir may not be needed if the config is retrieved from the running daemons.



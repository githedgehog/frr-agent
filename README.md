# frr-agent

* A daemon to reload FRR configurations.
* The daemon listens on a unix socket expecting configs and calls frr-reload to apply them.
* All parameters come from the cmd line. The only mandatory parameter is the address to bind the unix socket to.

```
Usage: frr-agent [OPTIONS] --sock-path <Unix socket bind path>

Options:
      --sock-path <Unix socket bind path>                         
      --outdir <Directory where received configs are stored>      
      --reloader <Full path to reloader (frr-reload.bin|py)>      
      --bindir <Directory of vtysh>                               
      --rundir <Directory of where frr-reload writes temp files>  
      --confdir <Directory of frr config files>                   
      --vtysock <vtysh sock (UNUSED atm)>                         
  -h, --help                                                      Print help
  -V, --version                                                   Print version
```

* confdir may not be needed if the config is retrieved from the running daemons.



#!/usr/bin/env python3
"""Private UI swarms resolve even public-looking bootstrap names locally."""
import socket
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'agent_ergonomics_audit/audit/regression_tests'))
import fixtures
import private_net
from fbtlog import Case,suite_status

if __name__=='__main__':
    private_net.enter()
    with Case('private_dns','no_host_resolver') as case,fixtures.Sandbox() as box:
        addresses={row[4][0] for row in socket.getaddrinfo('dht.transmissionbt.com',6881,type=socket.SOCK_DGRAM)}
        assert case.check(bool(addresses) and addresses<={'127.0.0.1','::1'},
                          'public-looking names resolve only through the isolated DNS fixture',str(sorted(addresses)))
    sys.exit(suite_status('private_dns'))

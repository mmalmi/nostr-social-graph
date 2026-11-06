import errno, json, os, pathlib, socket, subprocess, sys, threading

def denied():
    checks=[]
    for family,address in [(socket.AF_INET,('203.0.113.1',443)),(socket.AF_INET6,('2001:db8::1',443))]:
        for kind in (socket.SOCK_STREAM,socket.SOCK_DGRAM):
            with socket.socket(family,kind) as s:
                s.settimeout(.5)
                try:
                    if kind==socket.SOCK_DGRAM: s.sendto(b'namespace-negative-control',address)
                    else: s.connect(address)
                except OSError as e:
                    assert e.errno in (errno.ENETUNREACH,errno.EHOSTUNREACH,errno.EADDRNOTAVAIL,errno.EAFNOSUPPORT),repr(e)
                    checks.append({'family':family,'kind':kind,'errno':e.errno})
                else: raise AssertionError('External route escaped network namespace')
    return checks

if '--child' in sys.argv:
    print(json.dumps(denied()));sys.exit(0)
expected=['fixture0','lo'] if os.environ.get('FIPS_DUMMY')=='1' else ['lo']
assert sorted(os.listdir('/sys/class/net'))==expected,'Unexpected external interface'
status=pathlib.Path('/proc/self/status').read_text()
for field in ('CapEff','CapBnd'):assert int(next(line.split()[1] for line in status.splitlines() if line.startswith(field+':')),16)==0
routes=pathlib.Path('/proc/net/route').read_text();assert not routes.splitlines()[1:],'Unexpected IPv4 route; /32 dummy has only a local-table route'
routes6=pathlib.Path('/proc/net/ipv6_route').read_text();assert all(line.split()[-1]=='lo' for line in routes6.splitlines())
positive=[]
for kind in (socket.SOCK_STREAM,socket.SOCK_DGRAM):
    with socket.socket(socket.AF_INET,kind) as server:
        server.bind(('127.0.0.1',0));server.settimeout(2)
        if kind==socket.SOCK_STREAM:
            server.listen(1)
            def echo():
                c,_=server.accept()
                with c:c.sendall(c.recv(128))
        else:
            def echo():
                data,addr=server.recvfrom(128);server.sendto(data,addr)
        t=threading.Thread(target=echo);t.start()
        with socket.socket(socket.AF_INET,kind) as client:
            client.settimeout(2);client.connect(server.getsockname());client.sendall(b'loopback-positive');assert client.recv(128)==b'loopback-positive'
        t.join(2);assert not t.is_alive();positive.append(kind)
result={'interfaces':expected,'effectiveAndBoundingCapabilities':0,'route4':routes,'route6':routes6,'loopbackTcpUdp':positive,'externalDenied':denied(),'childDenied':json.loads(subprocess.check_output([sys.executable,__file__,'--child'],text=True,timeout=5)),'uid':os.getuid(),'pid':os.getpid(),'netNamespace':os.readlink('/proc/self/ns/net'),'pidNamespace':os.readlink('/proc/self/ns/pid')}
assert result['uid']!=0
pathlib.Path(sys.argv[1]).write_text(json.dumps(result,indent=2)+'\n');print('PASS loopback TCP/UDP, external IPv4/IPv6 TCP/UDP and inherited child denial')

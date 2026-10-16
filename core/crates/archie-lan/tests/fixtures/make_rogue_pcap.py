# Writes rogue-dhcp-and-ra.pcap (scapy): a Windows client, the router, an LLDP switch, a
# second device that answers DHCP and advertises itself as an IPv6 router, and then a DHCP
# starvation flood (one sender, 40 client addresses). Nothing is sent.
#   python3 -I make_rogue_pcap.py rogue-dhcp-and-ra.pcap
import sys
from scapy.all import Ether, IP, UDP, TCP, BOOTP, DHCP, IPv6, ICMPv6ND_RA, ICMPv6NDOptSrcLLAddr, Raw, wrpcap, ARP, load_layer, load_contrib
from scapy.layers.netbios import NBNSHeader, NBNSQueryRequest
load_layer("tls")
from scapy.layers.tls.all import TLS, TLSClientHello, TLS_Ext_ServerName, ServerName, TLS_Ext_SupportedGroups, TLS_Ext_SignatureAlgorithms
load_contrib("lldp")
from scapy.contrib.lldp import LLDPDUChassisID, LLDPDUPortID, LLDPDUTimeToLive, LLDPDUSystemName, LLDPDUEndOfLLDPDU
t0 = 1_791_749_000.0
router = "00:1a:11:00:00:01"      # Google-registered OUI: the real router
rogue = "02:00:5e:aa:bb:cc"       # locally administered: the rogue
win = "00:50:56:12:34:56"         # VMware OUI: a Windows VM
phone = "da:a1:19:00:00:07"       # random MAC
f = []
def at(p, s): p.time = t0 + s; f.append(p)
# A Windows client asks for an address (vendor class, option list), early in the capture.
at(Ether(src=win, dst="ff:ff:ff:ff:ff:ff")/IP(src="0.0.0.0", dst="255.255.255.255")/UDP(sport=68, dport=67)/
   BOOTP(chaddr=bytes.fromhex("005056123456"))/DHCP(options=[("message-type","discover"),("vendor_class_id",b"MSFT 5.0"),
   ("param_req_list",[1,3,6,15,31,33,43,44,46,47,119,121,249,252]),"end"]), 1)
# The router says who holds 192.168.1.1.
at(Ether(src=router, dst="ff:ff:ff:ff:ff:ff")/ARP(op=2, hwsrc=router, psrc="192.168.1.1", hwdst="ff:ff:ff:ff:ff:ff", pdst="192.168.1.255"), 1.5)
# A switch announces itself over LLDP.
sw = "00:1a:11:00:00:09"
at(Ether(src=sw, dst="01:80:c2:00:00:0e", type=0x88cc)/LLDPDUChassisID(subtype=4, id=sw)/LLDPDUPortID(subtype=3, id=sw)/
   LLDPDUTimeToLive(ttl=120)/LLDPDUSystemName(system_name="core-switch-01")/LLDPDUEndOfLLDPDU(), 1.7)
# The real router answers (learned during the learning window).
at(Ether(src=router, dst=win)/IP(src="192.168.1.1", dst="192.168.1.50", ttl=64)/UDP(sport=67, dport=68)/
   BOOTP(op=2, yiaddr="192.168.1.50")/DHCP(options=[("message-type","offer"),("server_id","192.168.1.1"),"end"]), 2)
# The Windows client opens a connection (TTL 128) with an HTTP User-Agent.
at(Ether(src=win, dst=router)/IP(src="192.168.1.50", dst="93.184.216.34", ttl=127)/TCP(sport=50000, dport=80, flags="S", window=64240), 3)
at(Ether(src=win, dst=router)/IP(src="192.168.1.50", dst="93.184.216.34", ttl=127)/TCP(sport=50000, dport=80, flags="PA")/
   Raw(b"GET / HTTP/1.1\r\nHost: example.com\r\nUser-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko)\r\n\r\n"), 4)
# NetBIOS name query from the Windows client.
at(Ether(src=win, dst="ff:ff:ff:ff:ff:ff")/IP(src="192.168.1.50", dst="192.168.1.255")/UDP(sport=137, dport=137)/
   NBNSHeader(NM_FLAGS=0x11)/NBNSQueryRequest(QUESTION_NAME="DESKTOP-ARCHIE", QUESTION_TYPE="NB"), 5)
# A TLS ClientHello from the Windows client: TShark computes its JA4.
hello = TLS(msg=[TLSClientHello(version=0x0303, ciphers=[0x1301, 0x1302, 0xc02b, 0xc02f],
    ext=[TLS_Ext_ServerName(servernames=[ServerName(servername=b"example.com")]),
         TLS_Ext_SupportedGroups(groups=[29, 23]), TLS_Ext_SignatureAlgorithms(sig_algs=[0x0403, 0x0804])])])
at(Ether(src=win, dst=router)/IP(src="192.168.1.50", dst="93.184.216.34", ttl=127)/TCP(sport=50001, dport=443, flags="PA", seq=1, ack=1)/hello, 6)
# After the learning window: a second device answers DHCP.
at(Ether(src=rogue, dst=win)/IP(src="192.168.1.66", dst="192.168.1.50")/UDP(sport=67, dport=68)/
   BOOTP(op=2, yiaddr="192.168.1.51")/DHCP(options=[("message-type","offer"),("server_id","192.168.1.66"),"end"]), 120)
# And offers itself as an IPv6 router.
at(Ether(src=rogue, dst="33:33:00:00:00:01")/IPv6(src="fe80::5eff:feaa:bbcc", dst="ff02::1", hlim=255)/ICMPv6ND_RA()/ICMPv6NDOptSrcLLAddr(lladdr=rogue), 121)
# Then one sender drains the pool: 40 discovers for 40 made-up client addresses in 8 s.
starver = "02:00:5e:dd:ee:ff"
for i in range(40):
    chaddr = bytes.fromhex("0200%08x" % (0xc0de0000 + i))
    at(Ether(src=starver, dst="ff:ff:ff:ff:ff:ff")/IP(src="0.0.0.0", dst="255.255.255.255")/UDP(sport=68, dport=67)/
       BOOTP(chaddr=chaddr, xid=i)/DHCP(options=[("message-type","discover"),"end"]), 130 + i * 0.2)
wrpcap(sys.argv[1], f)

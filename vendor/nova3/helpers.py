# VERSION: 1.55

# Author:
#  Christophe DUMEZ (chris@qbittorrent.org)

# Redistribution and use in source and binary forms, with or without
# modification, are permitted provided that the following conditions are met:
#
#    * Redistributions of source code must retain the above copyright notice,
#      this list of conditions and the following disclaimer.
#    * Redistributions in binary form must reproduce the above copyright
#      notice, this list of conditions and the following disclaimer in the
#      documentation and/or other materials provided with the distribution.
#    * Neither the name of the author nor the names of its contributors may be
#      used to endorse or promote products derived from this software without
#      specific prior written permission.
#
# THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
# AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
# IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
# ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
# LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
# CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
# SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
# INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
# CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
# ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
# POSSIBILITY OF SUCH DAMAGE.

import datetime
import gzip
import html
import io
import os
import socket
import ssl
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Mapping
from typing import Any, Optional, cast

import socks


def _getBrowserUserAgent() -> str:
    """ Disguise as browser to circumvent website blocking """

    # Firefox release calendar
    # https://whattrainisitnow.com/calendar/
    # https://wiki.mozilla.org/index.php?title=Release_Management/Calendar&redirect=no

    baseDate = datetime.date(2024, 4, 16)
    baseVersion = 125

    nowDate = datetime.date.today()
    nowVersion = baseVersion + ((nowDate - baseDate).days // 30)

    return f"Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:{nowVersion}.0) Gecko/20100101 Firefox/{nowVersion}.0"


_headers: dict[str, str] = {'User-Agent': _getBrowserUserAgent()}

# fbittorrent: bound every provider request. A stalled site must not hold a
# search open, and a huge body or gzip bomb must not exhaust memory. The
# default timeout also covers providers that open sockets themselves.
_TIMEOUT_SECS = 20
_MAX_BODY = 16 * 1024 * 1024
_MAX_DECOMPRESSED = 64 * 1024 * 1024
socket.setdefaulttimeout(_TIMEOUT_SECS)
_original_socket = socket.socket


def _enable_bound_proxy() -> None:
    """App-owned SOCKS5h: providers never invoke the machine's DNS resolver."""
    parts = urllib.parse.urlsplit(os.environ.get("qbt_socks_proxy", ""))
    if (parts.scheme != "socks5h" or parts.hostname != "127.0.0.1"
            or not parts.port or not parts.username or not parts.password):
        raise RuntimeError("Nova VPN proxy configuration is missing or invalid")
    proxy = (socks.SOCKS5, parts.hostname, parts.port, True,
             parts.username.encode(), parts.password.encode())
    socks.setdefaultproxy(socks.SOCKS5, parts.hostname, parts.port, True,
                          parts.username, parts.password)

    class BoundSocket(socks.socksocket):
        def __new__(cls, family=socket.AF_INET, type=socket.SOCK_STREAM, proto=0, fileno=None):
            # Multiprocessing uses local Unix sockets; these have no route.
            if family == socket.AF_UNIX:
                return _original_socket(family, type, proto, fileno=fileno)
            return super().__new__(cls)

        def __init__(self, family=socket.AF_INET, type=socket.SOCK_STREAM, proto=0, fileno=None):
            if family not in (socket.AF_INET, socket.AF_INET6) or type != socket.SOCK_STREAM or fileno is not None:
                raise OSError("Nova VPN proxy supports TCP CONNECT only")
            # The proxy endpoint is IPv4, even when the destination is IPv6.
            super().__init__(socket.AF_INET, type, proto)
            self.proxy = proxy
            self.settimeout(_TIMEOUT_SECS)

        def connect(self, address, *args, **kwargs):
            self.proxy = proxy
            return super().connect((address[0], address[1]), *args, **kwargs)

        def set_proxy(self, *args, **kwargs):
            raise OSError("Nova VPN proxy cannot be replaced")

        setproxy = set_proxy

    def create_connection(address, timeout=socket._GLOBAL_DEFAULT_TIMEOUT,
                          source_address=None, *, all_errors=False):
        stream = BoundSocket()
        try:
            if timeout is not socket._GLOBAL_DEFAULT_TIMEOUT:
                stream.settimeout(timeout)
            if source_address:
                # Providers do not own the outbound interface; the daemon does.
                raise OSError("Nova VPN proxy owns the source address")
            stream.connect(address)
            return stream
        except BaseException:
            stream.close()
            raise

    def getaddrinfo(host, port, family=0, type=0, proto=0, flags=0):
        # Libraries which resolve before connect must preserve the hostname for
        # SOCKS5h. No NSS, systemd-resolved or local DNS packets in this process.
        if type not in (0, socket.SOCK_STREAM):
            raise socket.gaierror("Nova VPN proxy supports TCP CONNECT only")
        if isinstance(host, bytes):
            host = host.decode("ascii")
        port = socket.getservbyname(port, "tcp") if isinstance(port, str) and not port.isdecimal() else int(port)
        return [(socket.AF_INET, socket.SOCK_STREAM, socket.IPPROTO_TCP, "", (host, port))]

    def no_local_dns(*args, **kwargs):
        raise socket.gaierror("Nova VPN DNS is remote; pass the hostname to connect")

    socket.socket = BoundSocket
    socket.create_connection = create_connection
    socket.getaddrinfo = getaddrinfo
    socket.gethostbyname = no_local_dns
    socket.gethostbyname_ex = no_local_dns
    socket.gethostbyaddr = no_local_dns
    socket.getnameinfo = no_local_dns


def _read_bounded(response: Any) -> bytes:
    """ Read at most _MAX_BODY bytes, and gunzip at most _MAX_DECOMPRESSED """
    data: bytes = response.read(_MAX_BODY + 1)
    if len(data) > _MAX_BODY:
        raise ValueError("response exceeds the provider body limit")
    if data[:2] == b'\x1f\x8b':
        with io.BytesIO(data) as compressedStream, gzip.GzipFile(fileobj=compressedStream) as gzipper:
            data = gzipper.read(_MAX_DECOMPRESSED + 1)
        if len(data) > _MAX_DECOMPRESSED:
            raise ValueError("decompressed response exceeds the provider limit")
    return data


def enable_socks_proxy(enable: bool) -> None:
    if os.environ.get("FBT_NOVA_BOUND") == "1":
        # A provider cannot opt out of the owner's binding by calling False.
        _enable_bound_proxy()
        return
    if enable:
        socksURL = os.environ.get("qbt_socks_proxy")
        if socksURL is not None:
            parts = urllib.parse.urlsplit(socksURL)
            resolveHostname = (parts.scheme == "socks4a") or (parts.scheme == "socks5h")
            if (parts.scheme == "socks4") or (parts.scheme == "socks4a"):
                socks.setdefaultproxy(socks.PROXY_TYPE_SOCKS4, parts.hostname, parts.port, resolveHostname)
                socket.socket = cast(type[socket.socket], socks.socksocket)  # type: ignore[misc]
            elif (parts.scheme == "socks5") or (parts.scheme == "socks5h"):
                socks.setdefaultproxy(socks.PROXY_TYPE_SOCKS5, parts.hostname, parts.port, resolveHostname, parts.username, parts.password)
                socket.socket = cast(type[socket.socket], socks.socksocket)  # type: ignore[misc]
        else:
            # the following code provide backward compatibility for older qbt versions
            # TODO: scheduled be removed with qbt >= 5.3
            legacySocksURL = os.environ.get("sock_proxy")
            if legacySocksURL is not None:
                legacySocksURL = f"socks5h://{legacySocksURL.strip()}"
                parts = urllib.parse.urlsplit(legacySocksURL)
                socks.setdefaultproxy(socks.PROXY_TYPE_SOCKS5, parts.hostname, parts.port, True, parts.username, parts.password)
                socket.socket = cast(type[socket.socket], socks.socksocket)  # type: ignore[misc]
    else:
        socket.socket = _original_socket  # type: ignore[misc]


# This is only provided for backward compatibility, new code should not use it
htmlentitydecode = html.unescape


def retrieve_url(url: str, custom_headers: Mapping[str, str] = {}, request_data: Optional[Any] = None, ssl_context: Optional[ssl.SSLContext] = None, unescape_html_entities: bool = True) -> str:
    """ Return the content of the url page as a string """

    request = urllib.request.Request(url, request_data, {**_headers, **custom_headers})
    try:
        response = urllib.request.urlopen(request, context=ssl_context, timeout=_TIMEOUT_SECS)
    except urllib.error.URLError as errno:
        print(f"Connection error: {errno.reason}", file=sys.stderr)
        return ""
    except OSError as error:
        print(f"Connection error: {error}", file=sys.stderr)
        return ""
    try:
        data = _read_bounded(response)
    except (OSError, EOFError, ValueError) as error:
        print(f"Connection error: {error}", file=sys.stderr)
        return ""

    charset = 'utf-8'
    try:
        charset = response.getheader('Content-Type', '').split('charset=', 1)[1]
    except IndexError:
        pass

    dataStr = data.decode(charset, 'replace')

    if unescape_html_entities:
        dataStr = html.unescape(dataStr)

    return dataStr


def download_file(url: str, referer: Optional[str] = None, ssl_context: Optional[ssl.SSLContext] = None) -> str:
    """ Download file at url and write it to a file, return the path to the file and the url """

    # Download url
    request = urllib.request.Request(url, headers=_headers)
    if referer is not None:
        request.add_header('referer', referer)
    response = urllib.request.urlopen(request, context=ssl_context, timeout=_TIMEOUT_SECS)
    data = _read_bounded(response)

    # Write it to a file
    fileHandle, path = tempfile.mkstemp()
    with os.fdopen(fileHandle, "wb") as file:
        file.write(data)

    # return file path
    return f"{path} {url}"

Fixed a test that failed now and then on Windows: the fake mail servers used in tests no longer guess between TLS and STARTTLS from timing, and listen on both IPv4 and IPv6 loopback.

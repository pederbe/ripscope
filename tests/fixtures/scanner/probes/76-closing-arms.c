/* Alternative heads written as consecutive groups share one body. */
#ifdef WIDE
int pick (long wide)
{
#endif
#ifdef NARROW
int pick (int narrow)
{
#endif
    return choose ();
}

int after_pick (void)
{
    return helper (0);
}

/* An arm that closes a brace, right before another conditional group. */
int retry_read (int fd)
{
    int got;
#ifdef ASYNC_IO
    do {
#endif
        got = read_some (fd);
#ifdef ASYNC_IO
    } while (got == PENDING);
#endif
#ifdef BLINDING
    free_blinding ();
#endif
    return got;
}

int after_retry (int x)
{
    return helper (x);
}


/* An arm that opens a brace, and a following group that closes it. */
int serve (int fd)
{
#ifdef SOCKET_ACTIVATION
    if (activated (fd))
        use_socket (fd);
    else {
#endif
#ifdef LOCAL_SOCKETS
        open_local (fd);
#ifdef SOCKET_ACTIVATION
    }
#endif
#else
    open_default (fd);
#endif
    return fd;
}

int after_serve (void)
{
    return helper (1);
}

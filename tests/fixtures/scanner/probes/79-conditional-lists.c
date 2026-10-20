/* A head whose parameter list is written in each arm of a conditional group. */
static int *
alloc
#ifdef KR_headers
	(k) int k;
#else
	(int k)
#endif
{
	int *block;
	block = grow (k);
	return block;
}

/* A list whose last parameters differ between the arms. */
int
connect_to (int port,
#if V5
	int flags, int extra)
#else
	int flags)
#endif
{
	return open_port (port, flags);
}

/* K&R declarations that differ between the arms. */
long
scale (n)
#ifdef WIDE
	long long n;
#else
	long n;
#endif
{
	return twice (n);
}

/* Alternative heads, each leaving its list open for the shared rest. */
const char *
#ifdef REPLACE
scan_version2 (const char *s, int strict,
#else
scan_version (const char *s, int strict,
#endif
	int *width)
{
	return check_width (s, width);
}

/* A name and a group whose first arm is no list keep their reading. */
int counter
#ifdef START
	= START
#endif
	;

int after (void)
{
	return tail_call ();
}

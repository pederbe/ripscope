/* Two bodies under one head, one per arm. */
void residual (int n)
#if SLOW
{
	slow_loop (n);
}
#elif MEDIUM
/* the middle way */
{
	medium_loop (n);
}
#else
{
	int unrolled;
	fast_loop (n);
}
#endif

/* A body whose arm is followed by a declaration keeps its end. */
#ifdef BOTH
int first (void)
{
	return one ();
}
#else
int second;
#endif

int after (void)
{
	return tail_call ();
}

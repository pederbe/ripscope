#if FLAG
int inside;
#endif /* comment
int phantom;
*/
int after;
void function(void) {
#if FLAG
    target();
#else /* comment
    phantom();
    ghost = 1;
*/
    real();
#endif
}
#if FLAG
#endif /* one */ /* two
phantom */ /* three
continued */
int final;
#if FLAG
#endif /* closing-line code stays outside the closed-tail profile
*/ int unchanged;

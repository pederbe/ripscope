/* leading comment with call(x) and int y; */
int before;
int spaced /* inline */ = 1;
int function_with_comment(void) /* between head and body */
{
	/* multi
	   line comment(z) */
	return value(); // trailing call(w)
}
int kr_comment(x) /** doc **/
int x;
{ g(4); }
/* later */

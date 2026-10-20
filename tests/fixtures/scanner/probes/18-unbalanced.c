}
int after_stray_close(void) { x(); }
}}
int another(void) { y(); }
int unclosed(void) {
	z();

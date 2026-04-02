package dev.evvie.waylandcraft.grabs;

/* Grab drop exception
 * 
 * Thrown when a PointerGrab is no longer valid.
 * Used as a way to remove stale pointer grabs that cannot operate any longer.
 */
public class GrabDroppedException extends Exception {
	
	private static final long serialVersionUID = 1L;
	
}

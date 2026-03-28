package dev.evvie.waylandcraft.desktop;

public class RawDesktopEntry {
	
	public final String appId;
	public final String name;
	public final String genericName;
	public final String exec;
	public final boolean execTerminal;
	public final String iconPath;
	
	public RawDesktopEntry(String appId, String name, String genericName, String exec, boolean execTerminal, String iconPath) {
		this.appId = appId;
		this.name = name;
		this.genericName = genericName;
		this.exec = exec;
		this.execTerminal = execTerminal;
		this.iconPath = iconPath;
	}
	
}

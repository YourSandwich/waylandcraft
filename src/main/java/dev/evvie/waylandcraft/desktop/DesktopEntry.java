package dev.evvie.waylandcraft.desktop;

import org.jetbrains.annotations.NotNull;
import org.jetbrains.annotations.Nullable;

import net.minecraft.resources.ResourceLocation;

public class DesktopEntry {
	
	public final @NotNull String appId;
	public final @Nullable String name;
	public final @Nullable String genericName;
	public final @Nullable String exec;
	public final boolean execTerminal;
	public final @Nullable ResourceLocation icon;
	
	public DesktopEntry(String appId, String name, String genericName, String exec, boolean execTerminal, ResourceLocation icon) {
		this.appId = appId;
		this.name = name;
		this.genericName = genericName;
		this.exec = exec;
		this.execTerminal = execTerminal;
		this.icon = icon;
	}
	
	@Override
	public String toString() {
		return "DesktopEntry [appId: " + appId + ", name: " + name + ", genericName: " + genericName + ", exec: '" + exec + "', execTerminal: " + execTerminal + ", icon: " + icon + "]";
	}
	
}

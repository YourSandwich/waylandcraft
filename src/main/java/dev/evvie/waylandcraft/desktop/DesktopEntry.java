package dev.evvie.waylandcraft.desktop;

import org.apache.commons.codec.digest.DigestUtils;
import org.jetbrains.annotations.NotNull;
import org.jetbrains.annotations.Nullable;

import dev.evvie.waylandcraft.WaylandCraft;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.AbstractTexture;
import net.minecraft.client.renderer.texture.TextureManager;
import net.minecraft.resources.ResourceLocation;

public class DesktopEntry {
	
	public @NotNull String appId;
	public @Nullable String name;
	public @Nullable String genericName;
	public @Nullable String exec;
	public boolean execTerminal;
	public boolean visible;
	protected String iconPath;
	private ResourceLocation icon = null;
	private boolean iconLoaded = false;
	
	public DesktopEntry(String appId, String name, String genericName, String exec, boolean execTerminal, boolean visible, String iconPath) {
		this.appId = appId;
		this.name = name;
		this.genericName = genericName;
		this.exec = exec;
		this.execTerminal = execTerminal;
		this.visible = visible;
		this.icon = null;
		this.iconPath = iconPath;
	}
	
	public ResourceLocation getIcon() {
		if(iconLoaded) return icon;
		iconLoaded = true;
		
		AbstractTexture texture = WaylandCraft.instance.xdgManager.tryLoadIcon(iconPath);
		if(texture == null) return null;
		
		TextureManager textureManager = Minecraft.getInstance().getTextureManager();
		icon = new ResourceLocation(WaylandCraft.MOD_ID, "icon_" + DigestUtils.sha1Hex(appId));
		textureManager.register(icon, texture);
		return icon;
	}
	
	@Override
	public String toString() {
		return "DesktopEntry [appId: " + appId + ", name: " + name + ", genericName: " + genericName + ", exec: '" + exec + "', execTerminal: " + execTerminal + ", visible: " + visible + ", iconPath" + iconPath + ", icon: " + icon + "]";
	}
	
}

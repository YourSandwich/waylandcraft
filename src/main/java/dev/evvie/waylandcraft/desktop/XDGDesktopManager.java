package dev.evvie.waylandcraft.desktop;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.util.ArrayList;

import org.apache.commons.codec.digest.DigestUtils;
import org.jetbrains.annotations.Nullable;

import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.platform.TextureUtil;

import dev.evvie.waylandcraft.WaylandCraft;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.AbstractTexture;
import net.minecraft.client.renderer.texture.TextureManager;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.resources.ResourceManager;

public class XDGDesktopManager {
	
	private final WaylandCraft wlc;
	private ArrayList<DesktopEntry> systemEntries;
	private ArrayList<DesktopEntry> localEntries = new ArrayList<DesktopEntry>();
	
	public XDGDesktopManager(WaylandCraft wlc) {
		this.wlc = wlc;
		
		this.loadSystemEntries();
	}
	
	private void loadSystemEntries() {
		systemEntries = new ArrayList<DesktopEntry>();
		for(RawDesktopEntry raw : wlc.bridge.loadSystemDesktopEntries()) {
			systemEntries.add(load(raw));
		}
	}
	
	private DesktopEntry load(RawDesktopEntry raw) {
		IconTexture icon = tryLoadIcon(raw.iconPath);
		ResourceLocation iconLocation = null;
		
		if(icon != null) {
			TextureManager textureManager = Minecraft.getInstance().getTextureManager();
			iconLocation = new ResourceLocation(WaylandCraft.MOD_ID, "icon_" + DigestUtils.sha1Hex(raw.appId));
			textureManager.register(iconLocation, icon);
		}
		
		return new DesktopEntry(raw.appId, raw.name, raw.genericName, raw.exec, raw.execTerminal, iconLocation);
	}
	
	public @Nullable DesktopEntry forAppId(String appId) {
		for(DesktopEntry entry : localEntries) {
			if(entry.appId.equals(appId)) return entry;
		}
		for(DesktopEntry entry : systemEntries) {
			if(entry.appId.equals(appId)) return entry;
		}
		return null;
	}
	
	public @Nullable String getName(String appId) {
		return forAppId(appId).name;
	}
	
	public @Nullable ResourceLocation getIcon(String appId) {
		return forAppId(appId).icon;
	}
	
	private String getExtension(File file) {
		String path = file.getAbsolutePath();
		int idx = path.lastIndexOf('.');
		if(idx < 0 || idx >= path.length() - 1) return "";
		
		return path.substring(idx + 1);
	}
	
	private IconTexture tryLoadIcon(String iconPath) {
		try {
			return loadIcon(iconPath);
		} catch(IOException e) {
			e.printStackTrace();
			return null;
		}
	}
	
	private IconTexture loadIcon(String iconPath) throws IOException {
		if(iconPath == null) return null;
		
		File iconFile = new File(iconPath);
		
		/* This "file type check" is valid because according to the Icon Theme Specification
		 * the extension has to be one of ".png", ".xpm" and ".svg" (lowercase) and the extension
		 * signals what type of file we should expect.
		 */
		if(!getExtension(iconFile).equals("png")) {
			System.err.println("Icon is not PNG!");
			return null;
		}
		
		return new IconTexture(iconFile);
	}
	
	public static class IconTexture extends AbstractTexture {
		
		private final NativeImage image;
		
		public IconTexture(File file) throws IOException {
			FileInputStream stream = new FileInputStream(file);
			image = NativeImage.read(stream);
			TextureUtil.prepareImage(getId(), image.getWidth(), image.getHeight());
			image.upload(0, 0, 0, false);
		}
		
		@Override
		public void load(ResourceManager resourceManager) throws IOException {
		}
		
		@Override
		public void close() {
			if(image != null) {
				image.close();
				releaseId();
			}
		}
		
	}
	
}
